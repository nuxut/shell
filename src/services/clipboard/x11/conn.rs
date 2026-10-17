//! Köprünün X11 tarafı (x11rb, saf Rust istemci): tek görünmez pencere, XFixes seçim olayları,
//! seçim dönüşümü (INCR alımı dahil) ve `SelectionRequest` yanıtları. Tek thread; X11 soketi
//! ve uyandırma soketi birlikte `poll` edilir, boşta uyanma yok.
//!
//! Bloklayan bekleyişlerin hepsi süreli: dönüşüm [`CONVERT_TIMEOUT`], tüm okuma
//! [`FETCH_TIMEOUT`], Wayland okuması `READ_TIMEOUT`. Bağlantı hatası (`ConnectionError`)
//! oturumu bitirir, çağıran yeniden bağlanır; X11 protokol hataları (ör. istekçinin penceresi
//! kayboldu) yalnızca o isteği düşürür. İçerik loglanmaz.

use std::collections::{HashMap, VecDeque};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError};
use x11rb::protocol::Event;
use x11rb::protocol::res::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::{self, ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
    SelectionRequestEvent, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};
use zeroize::Zeroizing;

use super::policy::{self, BridgeMode, Encoding, FetchPlan, Target, WDecision, XChange, XDecision};
use super::{Cmd, Inbox, Offer};
use crate::services::clipboard::{ClipboardSource, Gate, OfferId, READ_TIMEOUT};

/// Tek dönüşümün yanıt bekleme süresi.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(2);
/// Bir X11 seçiminin tümünü okuma süresi (birkaç dönüşüm).
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// Ertelenen olay kuyruğu sınırı (okuma sürerken gelenler; fazlası en eskiden düşer).
const MAX_DEFERRED: usize = 256;
/// Atom adı önbelleği sınırı (aşılınca boşaltılır).
const MAX_ATOMS: usize = 4096;
/// X11 isteğinin başlık payı (`ChangeProperty` başlığı < 32 bayt).
const REQUEST_HEADROOM: usize = 64;

/// Oturum hatası: bağlantı koptu mu, yoksa yalnızca bu istek mi başarısız.
#[derive(Debug, thiserror::Error)]
pub enum XError {
    #[error("bağlantı: {0}")]
    Connection(#[from] ConnectionError),
    #[error("X11 isteği reddedildi")]
    Protocol,
    #[error("bağlanılamadı: {0}")]
    Connect(#[from] ConnectError),
    #[error("kimlik tükendi")]
    Ids,
    #[error("X sunucusu bu kullanıcının Xwayland sunucusu değil")]
    NotOurs,
}

impl From<ReplyError> for XError {
    fn from(e: ReplyError) -> Self {
        match e {
            ReplyError::ConnectionError(c) => Self::Connection(c),
            ReplyError::X11Error(_) => Self::Protocol,
        }
    }
}

impl From<ReplyOrIdError> for XError {
    fn from(e: ReplyOrIdError) -> Self {
        match e {
            ReplyOrIdError::ConnectionError(c) => Self::Connection(c),
            ReplyOrIdError::X11Error(_) => Self::Protocol,
            ReplyOrIdError::IdsExhausted => Self::Ids,
        }
    }
}

impl XError {
    fn fatal(&self) -> bool {
        !matches!(self, Self::Protocol)
    }
}

type Res<T> = Result<T, XError>;

/// Oturum neden bitti.
pub enum Exit {
    Stop,
    Lost(XError),
}

/// Dosya tanıtıcılarından biri okunabilir olana ya da süre dolana dek bekler (hata: döner).
pub fn wait_readable(fds: &[&dyn AsFd], timeout: Option<Duration>) {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    let mut pfds: Vec<PollFd<'_>> = fds.iter().map(|f| PollFd::from_borrowed_fd(f.as_fd(), PollFlags::IN)).collect();
    let ts = timeout.map(|t| Timespec { tv_sec: t.as_secs().min(i64::MAX as u64) as i64, tv_nsec: i64::from(t.subsec_nanos()) });
    let _ = poll(&mut pfds, ts.as_ref());
}

struct Atoms {
    clipboard: u32,
    targets: u32,
    timestamp: u32,
    incr: u32,
    utf8: u32,
    /// Dönüşüm sonuçlarının yazıldığı özellik (kendi penceremizde).
    property: u32,
    /// Compositor'ın pencere yöneticisi seçimi (`WM_S<ekran>`).
    wm: u32,
}

/// X11 panosunu biz tutarken sunduğumuz Wayland teklifi.
struct Owned {
    offer: OfferId,
    targets: Vec<(u32, Target)>,
    /// Sahipliğin sunucu zamanı (kendi XFixes olayımızdan; bilinmiyorsa 0).
    time: u32,
}

pub struct Session {
    conn: RustConnection,
    window: u32,
    /// Kök pencere (seçim yanıtı oraya yazılmaz).
    root: u32,
    atoms: Atoms,
    names: HashMap<u32, String>,
    ids: HashMap<String, u32>,
    resource_mask: u32,
    max_request: usize,
    source: Arc<dyn ClipboardSource>,
    /// Gizlilik kapısı (oturum kilidi, hariç tutulan uygulama): kapalıyken iki yönde de içerik
    /// okunmaz ve taşınmaz.
    gate: Arc<Gate>,
    /// Köprü kipi: `auto`da yalnız konuk ajanına ait pencereler yetkili ([`policy::client_allowed`]).
    mode: BridgeMode,
    owned: Option<Owned>,
    /// Son gerçek X11 sahibi değişimi (compositor yansıtması sezgisi).
    last_x: Option<XChange>,
    /// Son X11 → Wayland içeriğinin özeti ve o zamandan beri Wayland'de başka seçim oldu mu.
    last_set: Option<u64>,
    wayland_changed: bool,
    /// Son Wayland teklifi (yeniden bağlanınca yeniden sunulur).
    latest: Option<Offer>,
    deferred: VecDeque<Event>,
    /// Aynı isteğe tekrar tekrar okumamak için son Wayland okuması.
    cache: Option<(OfferId, String, Zeroizing<Vec<u8>>)>,
    /// Ajanın penceresine yazılan, henüz silinmemiş yanıt özellikleri.
    cleanup: policy::ReplyCleanup,
}

impl Session {
    pub fn connect(display: &str, mode: BridgeMode, source: Arc<dyn ClipboardSource>, gate: Arc<Gate>) -> Res<Self> {
        let (conn, screen) = RustConnection::connect(Some(display))?;
        // Sunucu bu kullanıcının Xwayland'i mi (soketin karşı ucunun kimliği): başka kullanıcının
        // X sunucusuna, ya da panoyu ağa/başka ekrana taşıyabilen sunuculara (aynı kullanıcının
        // Xvnc/Xpra/Xephyr'i bile) pano verilmez.
        let peer = rustix::net::sockopt::socket_peercred(conn.stream()).map_err(|_| XError::NotOurs)?;
        let comm = std::fs::read_to_string(format!("/proc/{}/comm", peer.pid.as_raw_nonzero())).unwrap_or_default();
        if !policy::is_own_xwayland(&comm, peer.uid.as_raw(), rustix::process::getuid().as_raw()) {
            return Err(XError::NotOurs);
        }
        let setup = conn.setup();
        let root = setup.roots.get(screen).ok_or(XError::Protocol)?.root;
        let resource_mask = setup.resource_id_mask;
        let window = conn.generate_id()?;
        conn.create_window(
            0,
            window,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().override_redirect(1).event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        let intern = |name: &str| conn.intern_atom(false, name.as_bytes());
        let cookies = [
            intern("CLIPBOARD")?,
            intern("TARGETS")?,
            intern("TIMESTAMP")?,
            intern("INCR")?,
            intern("UTF8_STRING")?,
            intern("_NUXUT_CLIPBOARD")?,
            intern(&format!("WM_S{screen}"))?,
        ];
        let mut a = [0u32; 7];
        for (slot, c) in a.iter_mut().zip(cookies) {
            *slot = c.reply()?.atom;
        }
        let atoms = Atoms { clipboard: a[0], targets: a[1], timestamp: a[2], incr: a[3], utf8: a[4], property: a[5], wm: a[6] };
        conn.xfixes_query_version(5, 0)?.reply()?;
        conn.xfixes_select_selection_input(
            window,
            atoms.clipboard,
            SelectionEventMask::SET_SELECTION_OWNER | SelectionEventMask::SELECTION_WINDOW_DESTROY | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?;
        conn.flush()?;
        let max_request = conn.maximum_request_bytes();
        Ok(Self {
            conn,
            window,
            root,
            atoms,
            names: HashMap::new(),
            ids: HashMap::new(),
            resource_mask,
            max_request,
            source,
            gate,
            mode,
            owned: None,
            last_x: None,
            last_set: None,
            wayland_changed: true,
            latest: None,
            deferred: VecDeque::new(),
            cache: None,
            cleanup: policy::ReplyCleanup::default(),
        })
    }

    pub fn into_latest(self) -> Option<Offer> {
        self.latest
    }

    /// Olay döngüsü. `latest`: bağlanmadan önce gelen son Wayland teklifi.
    pub fn run(&mut self, inbox: &Inbox, latest: Option<Offer>) -> Exit {
        if let Some(o) = latest
            && let Err(e) = self.on_offer(o)
            && e.fatal()
        {
            return Exit::Lost(e);
        }
        loop {
            inbox.drain_wake();
            loop {
                let r = match inbox.rx.try_recv() {
                    Ok(Cmd::Offered(o)) => self.on_offer(o),
                    Ok(Cmd::Cleared) => {
                        self.latest = None;
                        self.wayland_changed = true;
                        self.release()
                    }
                    Ok(Cmd::Stop) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let _ = self.release();
                        let _ = self.conn.flush();
                        return Exit::Stop;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                };
                if let Err(e) = r
                    && e.fatal()
                {
                    return Exit::Lost(e);
                }
            }
            loop {
                let ev = match self.next_event() {
                    Ok(Some(ev)) => ev,
                    Ok(None) => break,
                    Err(e) => return Exit::Lost(e),
                };
                if let Err(e) = self.on_event(ev)
                    && e.fatal()
                {
                    return Exit::Lost(e);
                }
            }
            for (w, p) in self.cleanup.due(Instant::now()) {
                // Pencere kaybolduysa hata olayı gelir, yok sayılır.
                if let Err(e) = self.conn.delete_property(w, p) {
                    return Exit::Lost(e.into());
                }
            }
            if let Err(e) = self.conn.flush() {
                return Exit::Lost(e.into());
            }
            wait_readable(&[self.conn.stream(), &inbox.wake], self.cleanup.next_in(Instant::now()));
        }
    }

    fn next_event(&mut self) -> Res<Option<Event>> {
        if let Some(ev) = self.deferred.pop_front() {
            return Ok(Some(ev));
        }
        Ok(self.conn.poll_for_event()?)
    }

    fn on_event(&mut self, ev: Event) -> Res<()> {
        match ev {
            Event::XfixesSelectionNotify(e) if e.selection == self.atoms.clipboard => self.on_owner(e),
            Event::SelectionRequest(e) => self.on_request(&e),
            Event::SelectionClear(e) if e.selection == self.atoms.clipboard && e.owner == self.window => {
                self.owned = None;
                self.cache = None;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // --- Wayland → X11 ---

    fn on_offer(&mut self, offer: Offer) -> Res<()> {
        let marked = offer.mimes.iter().any(|m| m == policy::BRIDGE_MIME);
        let decision = policy::decide_wayland(
            &offer.mimes,
            offer.flags.sensitive,
            offer.flags.truncated,
            offer.blocked,
            self.last_x.as_ref(),
            offer.at,
        );
        if !marked {
            self.wayland_changed = true;
        }
        self.latest = Some(offer.clone());
        match decision {
            WDecision::Ignore => Ok(()),
            WDecision::Release => {
                tracing::debug!("pano köprüsü: Wayland teklifi X11'e taşınmadı");
                self.release()
            }
            WDecision::Claim(targets) => {
                let mut with_atoms = Vec::with_capacity(targets.len());
                for t in targets {
                    with_atoms.push((self.atom(&t.name)?, t));
                }
                self.owned = Some(Owned { offer: offer.id, targets: with_atoms, time: 0 });
                self.cache = None;
                self.conn.set_selection_owner(self.window, self.atoms.clipboard, CURRENT_TIME)?;
                Ok(())
            }
        }
    }

    /// Bizim X11 seçimimizi bırakır (sahibi biz değilsek dokunmaz).
    fn release(&mut self) -> Res<()> {
        self.cache = None;
        if self.owned.take().is_none() {
            return Ok(());
        }
        let owner = self.conn.get_selection_owner(self.atoms.clipboard)?.reply()?.owner;
        if owner == self.window {
            self.conn.set_selection_owner(NONE, self.atoms.clipboard, CURRENT_TIME)?;
        }
        Ok(())
    }

    fn on_request(&mut self, e: &SelectionRequestEvent) -> Res<()> {
        let property = if e.property == NONE { e.target } else { e.property };
        let ok = match self.answer(e, property) {
            Ok(ok) => ok,
            Err(err) if err.fatal() => return Err(err),
            Err(_) => false,
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if ok { property } else { NONE },
        };
        self.conn.send_event(false, e.requestor, EventMask::NO_EVENT, notify)?;
        Ok(())
    }

    /// `window` (yanıtın yazılacağı pencere ya da pano sahibinin penceresi) köprüyü kullanabilir
    /// mi ([`policy::client_allowed`]): `auto`'da pencerenin istemci süreci (XRes) bu kullanıcının
    /// konuk ajanı olmalı. Yalnız "bir ajan çalışıyor" yetmez; pencere de ajana ait olmalı — böylece
    /// yanıt yalnız ajanın penceresine yazılır. Süreç kimliği okunamazsa yetki yok.
    fn authorized(&self, window: u32) -> Res<bool> {
        if self.mode == BridgeMode::On {
            return Ok(true);
        }
        let agent = self.client_pid(window)?.is_some_and(|pid| super::own_guest_agent(pid).is_some());
        Ok(policy::client_allowed(self.mode, agent))
    }

    /// Pencerenin sahibi istemcinin yerel süreç kimliği (XRes `QueryClientIds`). XRes yoksa,
    /// pencere kaybolduysa ya da istemci uzaksa `None` (yetki yok sayılır).
    fn client_pid(&self, window: u32) -> Res<Option<u32>> {
        let spec = res::ClientIdSpec { client: window, mask: res::ClientIdMask::LOCAL_CLIENT_PID };
        let cookie = match self.conn.res_query_client_ids(&[spec]) {
            Ok(c) => c,
            Err(ConnectionError::UnsupportedExtension) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let reply = match cookie.reply() {
            Ok(r) => r,
            Err(ReplyError::X11Error(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let pid_mask = u32::from(res::ClientIdMask::LOCAL_CLIENT_PID);
        Ok(reply.ids.iter().find(|v| u32::from(v.spec.mask) & pid_mask != 0).and_then(|v| v.value.first().copied()))
    }

    /// İsteği yanıtlar (özelliği yazar); reddedilecekse `false`.
    fn answer(&mut self, e: &SelectionRequestEvent, property: u32) -> Res<bool> {
        if self.owned.is_none() || e.owner != self.window || e.selection != self.atoms.clipboard {
            return Ok(false);
        }
        // Ön denetim (policy::check_request): kilit her istek anında ve ilk iş — kilitten önce
        // yapılmış teklif de kilitliyken okunmaz, tür listesi bile verilmez; önbellekteki içerik
        // bırakılır (sıfırlanarak). Yanıt yalnız isteyenin penceresine, seçim özelliğine yazılır.
        let locked = self.gate.locked();
        let name = if locked { None } else { self.atom_name(property)? };
        if let Err(why) = policy::check_request(locked, e.requestor, self.root, self.window, name.as_deref()) {
            if why == policy::Reject::Locked {
                self.cache = None;
            }
            tracing::debug!(?why, "pano köprüsü: X11 isteği reddedildi");
            return Ok(false);
        }
        // Yetkisiz (odaksız, ajan olmayan) istemciye tür listesi dahil hiçbir şey verilmez.
        if !self.authorized(e.requestor)? {
            tracing::debug!("pano köprüsü: odaksız X11 istemcisinin isteği reddedildi");
            return Ok(false);
        }
        let Some(owned) = &self.owned else { return Ok(false) };
        if e.time != CURRENT_TIME && owned.time != 0 && e.time < owned.time {
            return Ok(false);
        }
        if e.target == self.atoms.targets {
            let mut list = vec![self.atoms.targets, self.atoms.timestamp];
            list.extend(owned.targets.iter().map(|(a, _)| *a));
            self.conn.change_property32(PropMode::REPLACE, e.requestor, property, AtomEnum::ATOM, &list)?;
            return Ok(true);
        }
        if e.target == self.atoms.timestamp {
            let time = owned.time;
            self.conn.change_property32(PropMode::REPLACE, e.requestor, property, AtomEnum::INTEGER, &[time])?;
            return Ok(true);
        }
        let Some((_, target)) = owned.targets.iter().find(|(a, _)| *a == e.target) else { return Ok(false) };
        let target = target.clone();
        let offer = owned.offer;
        let data = if target.encoding == Encoding::Marker {
            Zeroizing::new(Vec::new())
        } else {
            let Some(raw) = self.read_wayland(offer, &target.mime) else { return Ok(false) };
            match policy::convert(&raw, target.encoding) {
                Some(d) => Zeroizing::new(d),
                None => return Ok(false),
            }
        };
        // Okuma sürerken (en çok READ_TIMEOUT) kilitlendiyse verilmez.
        if self.gate.locked() {
            self.cache = None;
            return Ok(false);
        }
        // INCR gönderimi yok: tek istekte sığmayan içerik reddedilir.
        if data.len() + REQUEST_HEADROOM > self.max_request {
            tracing::debug!(len = data.len(), "pano köprüsü: içerik X11 isteğine sığmıyor");
            return Ok(false);
        }
        let kind = if target.name == "TEXT" { self.atoms.utf8 } else { e.target };
        self.conn.change_property8(PropMode::REPLACE, e.requestor, property, kind, &data)?;
        // Ajan okuyup silmezse içerik X11'de kalmasın: kısa süre sonra köprü siler.
        if let Some((w, p)) = self.cleanup.written(e.requestor, property, Instant::now()) {
            self.conn.delete_property(w, p)?;
        }
        Ok(true)
    }

    fn read_wayland(&mut self, offer: OfferId, mime: &str) -> Option<Zeroizing<Vec<u8>>> {
        if let Some((o, m, d)) = &self.cache
            && *o == offer
            && m == mime
        {
            return Some(Zeroizing::new(d.to_vec()));
        }
        match self.source.read(offer, mime, policy::MAX_BYTES, READ_TIMEOUT) {
            Ok(d) => {
                self.cache = Some((offer, mime.to_owned(), Zeroizing::new(d.to_vec())));
                Some(d)
            }
            Err(e) => {
                tracing::debug!(error = %e, "pano köprüsü: Wayland teklifi okunamadı");
                None
            }
        }
    }

    // --- X11 → Wayland ---

    fn on_owner(&mut self, e: xfixes::SelectionNotifyEvent) -> Res<()> {
        if e.owner == self.window {
            if let Some(o) = &mut self.owned {
                o.time = e.selection_timestamp;
            }
            return Ok(());
        }
        // Sahipliği kaybettiğimizi `SelectionClear` bildirir; burada `owned`'a dokunulmaz: olay,
        // biz sahiplik almadan önceki bir değişimden kalmış olabilir (ör. önceki sahip kapandı).
        if e.owner == NONE {
            return Ok(());
        }
        // Arkadan daha yeni bir sahip değişimi geldiyse bu atlanır.
        let clipboard = self.atoms.clipboard;
        if self.deferred.iter().any(|ev| matches!(ev, Event::XfixesSelectionNotify(n) if n.selection == clipboard)) {
            return Ok(());
        }
        // Eskimiş olay: şu anki sahip başkasıysa (ör. biz aldıysak) okunmaz — kendimize
        // dönüşüm isteği göndermek bekleyip zaman aşımına düşerdi.
        if self.conn.get_selection_owner(clipboard)?.reply()?.owner != e.owner {
            return Ok(());
        }
        let wm_owner = self.conn.get_selection_owner(self.atoms.wm)?.reply()?.owner;
        if policy::owned_by_compositor(e.owner, Some(wm_owner), self.resource_mask) {
            return Ok(());
        }
        self.last_x = Some(XChange { at: Instant::now(), targets: Vec::new() });
        // Kilitliyken ya da hariç tutulan uygulama etkinken X11 seçimi okunmaz (hedef listesi
        // dahil). Odaktaki X11 uygulamasının kopyası Hyprland'in kendi eşitlemesiyle yine geçer.
        if self.gate.blocks_transfer() {
            tracing::debug!("pano köprüsü: kapı kapalı; X11 seçimi okunmadı");
            return Ok(());
        }
        // Odaksız, ajan olmayan X11 istemcisi Wayland panosuna seçim koyamaz (Hyprland kuralı).
        if !self.authorized(e.owner)? {
            tracing::debug!("pano köprüsü: odaksız X11 istemcisinin seçimi taşınmadı");
            return Ok(());
        }
        let deadline = Instant::now() + FETCH_TIMEOUT;
        let Some(targets) = self.fetch_targets(deadline)? else { return Ok(()) };
        if let Some(x) = &mut self.last_x {
            x.targets.clone_from(&targets);
        }
        let plan = match policy::plan_fetch(&targets) {
            XDecision::Fetch(p) => p,
            other => {
                tracing::debug!(decision = ?std::mem::discriminant(&other), "pano köprüsü: X11 seçimi taşınmadı");
                return Ok(());
            }
        };
        let data = self.fetch(&plan, deadline)?;
        if data.is_empty() {
            return Ok(());
        }
        let digest = policy::digest(&data);
        if self.last_set == Some(digest) && !self.wayland_changed {
            return Ok(());
        }
        // Okuma sürerken oturum kilitlendiyse konmaz.
        if self.gate.locked() {
            return Ok(());
        }
        let mut data = data;
        data.push((policy::BRIDGE_MIME.to_owned(), Arc::from(&[][..])));
        match self.source.set_selection(data) {
            Ok(()) => {
                self.last_set = Some(digest);
                self.wayland_changed = false;
            }
            Err(e) => tracing::debug!(error = %e, "pano köprüsü: Wayland seçimi konamadı"),
        }
        Ok(())
    }

    fn fetch_targets(&mut self, deadline: Instant) -> Res<Option<Vec<String>>> {
        let max = (policy::MAX_TARGETS + 1) * 4;
        let Some(bytes) = self.convert(self.atoms.targets, max, deadline)? else { return Ok(None) };
        if bytes.len() % 4 != 0 || bytes.len() / 4 > policy::MAX_TARGETS {
            return Ok(None);
        }
        let atoms: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
        let mut names = Vec::with_capacity(atoms.len());
        for a in atoms {
            if let Some(n) = self.atom_name(a)? {
                names.push(n);
            }
        }
        Ok(Some(names))
    }

    /// Plandaki türleri okur; toplam [`policy::MAX_BYTES`].
    fn fetch(&mut self, plan: &FetchPlan, deadline: Instant) -> Res<Vec<(String, Arc<[u8]>)>> {
        let mut out: Vec<(String, Arc<[u8]>)> = Vec::new();
        let mut budget = policy::MAX_BYTES;
        if let Some((name, enc)) = &plan.text {
            let atom = self.atom(name)?;
            if let Some(raw) = self.convert(atom, budget, deadline)?
                && let Some(text) = policy::convert(&raw, *enc).map(Zeroizing::new)
                && std::str::from_utf8(&text).is_ok()
                && !text.is_empty()
            {
                budget = budget.saturating_sub(text.len());
                let shared: Arc<[u8]> = Arc::from(&text[..]);
                for m in policy::TEXT_OFFER {
                    out.push(((*m).to_owned(), shared.clone()));
                }
            }
        }
        for name in plan.image.iter().chain(&plan.extra) {
            if budget == 0 || Instant::now() >= deadline {
                break;
            }
            let atom = self.atom(name)?;
            if let Some(raw) = self.convert(atom, budget, deadline)?
                && !raw.is_empty()
            {
                budget = budget.saturating_sub(raw.len());
                out.push((name.clone(), Arc::from(&raw[..])));
            }
        }
        Ok(out)
    }

    /// `CLIPBOARD`'ı `target`'a dönüştürür ve en çok `max` bayt okur (INCR dahil). Sahibi
    /// reddederse, süre dolarsa ya da sınır aşılırsa `None`.
    fn convert(&mut self, target: u32, max: usize, deadline: Instant) -> Res<Option<Zeroizing<Vec<u8>>>> {
        let (win, prop, clipboard) = (self.window, self.atoms.property, self.atoms.clipboard);
        self.conn.delete_property(win, prop)?;
        self.conn.convert_selection(win, clipboard, target, prop, CURRENT_TIME)?;
        self.conn.flush()?;
        let until = deadline.min(Instant::now() + CONVERT_TIMEOUT);
        let notify = self.wait_for(until, |ev| {
            matches!(ev, Event::SelectionNotify(n) if n.requestor == win && n.selection == clipboard && n.target == target)
        })?;
        let Some(Event::SelectionNotify(n)) = notify else { return Ok(None) };
        if n.property == NONE {
            return Ok(None);
        }
        let words = u32::try_from(max / 4 + 1).unwrap_or(u32::MAX);
        let reply = self.conn.get_property(false, win, prop, AtomEnum::ANY, 0, words)?.reply()?;
        if reply.type_ == self.atoms.incr {
            self.conn.delete_property(win, prop)?;
            self.conn.flush()?;
            return self.receive_incr(max, until);
        }
        self.conn.delete_property(win, prop)?;
        if reply.bytes_after > 0 || reply.value.len() > max {
            let mut v = reply.value;
            zeroize::Zeroize::zeroize(&mut v);
            return Ok(None);
        }
        Ok(Some(Zeroizing::new(reply.value)))
    }

    /// INCR: her `NewValue` bir parça; boş parça bitiş.
    fn receive_incr(&mut self, max: usize, until: Instant) -> Res<Option<Zeroizing<Vec<u8>>>> {
        let (win, prop) = (self.window, self.atoms.property);
        let mut buf = Zeroizing::new(Vec::new());
        loop {
            let ev = self.wait_for(until, |ev| {
                matches!(ev, Event::PropertyNotify(p) if p.window == win && p.atom == prop && p.state == Property::NEW_VALUE)
            })?;
            if ev.is_none() {
                return Ok(None);
            }
            let left = max.saturating_sub(buf.len());
            let words = u32::try_from(left / 4 + 1).unwrap_or(u32::MAX);
            let mut reply = self.conn.get_property(true, win, prop, AtomEnum::ANY, 0, words)?.reply()?;
            self.conn.flush()?;
            if reply.value.is_empty() {
                return Ok(Some(buf));
            }
            if reply.bytes_after > 0 || reply.value.len() > left {
                zeroize::Zeroize::zeroize(&mut reply.value);
                return Ok(None);
            }
            // Büyürken eski tampon sıfırlanır (yeniden ayırmada kopya kalmaz).
            if buf.len() + reply.value.len() > buf.capacity() {
                let mut next = Zeroizing::new(Vec::with_capacity((buf.len() + reply.value.len()).max(buf.capacity() * 2).min(max)));
                next.extend_from_slice(&buf);
                buf = next;
            }
            buf.extend_from_slice(&reply.value);
            zeroize::Zeroize::zeroize(&mut reply.value);
        }
    }

    /// `pred`'e uyan olayı `until`'e dek bekler; diğerleri sonraya ertelenir.
    fn wait_for(&mut self, until: Instant, pred: impl Fn(&Event) -> bool) -> Res<Option<Event>> {
        loop {
            while let Some(ev) = self.conn.poll_for_event()? {
                if pred(&ev) {
                    return Ok(Some(ev));
                }
                if self.deferred.len() == MAX_DEFERRED {
                    self.deferred.pop_front();
                }
                self.deferred.push_back(ev);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            wait_readable(&[self.conn.stream()], Some(left));
        }
    }

    fn atom(&mut self, name: &str) -> Res<u32> {
        if let Some(&a) = self.ids.get(name) {
            return Ok(a);
        }
        let a = self.conn.intern_atom(false, name.as_bytes())?.reply()?.atom;
        self.remember(a, name);
        Ok(a)
    }

    fn atom_name(&mut self, atom: u32) -> Res<Option<String>> {
        if let Some(n) = self.names.get(&atom) {
            return Ok(Some(n.clone()));
        }
        let reply = match self.conn.get_atom_name(atom)?.reply() {
            Ok(r) => r,
            Err(ReplyError::X11Error(_)) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if reply.name.len() > policy::MAX_NAME_LEN {
            return Ok(Some(String::new()));
        }
        let Ok(name) = String::from_utf8(reply.name) else { return Ok(Some(String::new())) };
        self.remember(atom, &name);
        Ok(Some(name))
    }

    fn remember(&mut self, atom: u32, name: &str) {
        if self.names.len() >= MAX_ATOMS {
            self.names.clear();
            self.ids.clear();
        }
        self.names.insert(atom, name.to_owned());
        self.ids.insert(name.to_owned(), atom);
    }
}

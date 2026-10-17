//! VM pano köprüsünün saf kararları (IO yok, birim testli): konuk ajanı algılama, X11 hedef
//! adı ↔ Wayland MIME eşlemesi, okunacak türlerin seçimi, döngü önleme (işaret, compositor'ın
//! kendi yansıtması, aynı içerik). Tasarım: `docs/design/vm-clipboard.md`.

use std::time::{Duration, Instant};

/// Köprünün X11 → Wayland yönünde koyduğu seçimlerin işaret türü. Wayland teklifinde ya da
/// X11 hedef listesinde görülürse o seçim köprüden gelmiştir: geri yansıtılmaz. İçerik taşımaz
/// (boş bayt). Pano geçmişi bu işareti taşıyan kendi teklifimizi yine okur (ana makineden
/// kopyalanan da geçmişe girsin).
pub const BRIDGE_MIME: &str = "application/x-nuxut-x11-bridge";

/// Köprünün bir yönde taşıdığı toplam içerik sınırı (bayt). X11'e tek `ChangeProperty` ile
/// yazılır (INCR gönderimi yok); sınır ayrıca bağlantının en büyük isteğiyle kısılır.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// X11 → Wayland'de metin ve görsel dışında taşınan en çok ek tür (ör. `text/html`,
/// `text/uri-list`). Her biri ayrı bir dönüşüm isteğidir.
pub const MAX_EXTRA: usize = 4;
/// X11 hedef listesinde kabul edilen en çok öğe; fazlası kırpılmaz, liste tümüyle atlanır.
pub const MAX_TARGETS: usize = 64;
/// Hedef/MIME adının en çok uzunluğu (bayt).
pub const MAX_NAME_LEN: usize = 256;
/// Gerçek bir X11 istemcisi panoyu aldıktan sonra bu süre içinde gelen işaretsiz Wayland
/// teklifi compositor'ın kendi X11 → Wayland yansıtması sayılır (geri X11'e yansıtılmaz).
pub const MIRROR_WINDOW: Duration = Duration::from_secs(1);

/// Bilinen VM konuk ajanları (`/proc/<pid>/comm`, en çok 15 bayt). Hepsi panoyu yalnızca X11
/// üzerinden konuşur.
pub const GUEST_AGENTS: &[&str] = &[
    "spice-vdagent", // QEMU/KVM SPICE (virt-manager, GNOME Boxes)
    "vmtoolsd",      // VMware open-vm-tools (`vmtoolsd -n vmusr`)
    "vmware-user",   // eski VMware araçları
    "VBoxClient",    // VirtualBox konuk eklentileri (`VBoxClient --clipboard`)
];

/// `[clipboard] x11_bridge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BridgeMode {
    /// Yalnızca sanal makinede bir konuk ajanı çalışırken.
    #[default]
    Auto,
    /// Her zaman (ör. X11 pano araçları; Hyprland'in odak korumasını kaldırır).
    On,
    Off,
}

impl BridgeMode {
    pub const ALL: [Self; 3] = [Self::Auto, Self::On, Self::Off];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// `/proc/cpuinfo` bir hipervizör altında çalıştığımızı söylüyor mu (x86 `hypervisor` bayrağı).
pub fn cpuinfo_has_hypervisor(cpuinfo: &str) -> bool {
    cpuinfo
        .lines()
        .filter(|l| l.starts_with("flags"))
        .any(|l| l.split_once(':').is_some_and(|(_, f)| f.split_ascii_whitespace().any(|w| w == "hypervisor")))
}

/// Süreç adı (`comm`, sondaki satır sonu dahil olabilir) bir konuk ajanı mı.
pub fn is_guest_agent(comm: &str) -> bool {
    let comm = comm.trim_end();
    GUEST_AGENTS.contains(&comm)
}

/// Konuk ajanı sayılır mı: adı listede **ve** sahibi bu kullanıcı. Başka bir yerel kullanıcının
/// aynı adlı süreci köprüyü açamaz ya da ajan yetkisi alamaz (`comm` herkesçe seçilebilir).
pub fn is_own_guest_agent(comm: &str, owner_uid: u32, our_uid: u32) -> bool {
    owner_uid == our_uid && is_guest_agent(comm)
}

/// Bağlanılan X sunucusu (soketin karşı ucu) bu kullanıcının Xwayland'i mi (`comm` + uid).
pub fn is_own_xwayland(comm: &str, peer_uid: u32, our_uid: u32) -> bool {
    peer_uid == our_uid && comm.trim_end() == "Xwayland"
}

/// `DISPLAY` yerel bir X sunucusu mu (`:N`, `:N.S`, `unix:N[.S]`); numarayı döner. Ana makine
/// adı / TCP biçimi (`host:0`, `localhost:10.0` — SSH X yönlendirmesi dahil) reddedilir: köprü
/// panoyu ağdaki ya da başkasının X sunucusuna taşımaz.
pub fn local_display(display: &str) -> Option<u32> {
    let rest = display.strip_prefix("unix:").or_else(|| display.strip_prefix(':'))?;
    let (num, screen) = rest.split_once('.').unwrap_or((rest, "0"));
    let digits = |s: &str| !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit());
    (digits(num) && digits(screen)).then(|| num.parse().ok()).flatten()
}

/// `SelectionRequest` yanıtının yazılacağı özellik adı kabul edilir mi. İsteyen pencereyi ve
/// özelliği istemci seçer; köprü başkasının yerine pencere yöneticisi / masaüstü / kaynak
/// veritabanı özelliklerini (`WM_*`, `_NET_*`, `RESOURCE_MANAGER`, `CUT_BUFFER*`…) yazan bir
/// araca dönüşmesin. Seçim istemcileri kendi adlarını kullanır (`XCLIP_OUT`, `GDK_SELECTION`,
/// `_QT_SELECTION`, `VDAGENT_SELECTION`…).
pub fn property_allowed(name: &str) -> bool {
    const PREFIXES: &[&str] = &["WM_", "_NET_", "_MOTIF_", "_XEMBED", "_XSETTINGS", "_GTK_", "_KDE_", "CUT_BUFFER", "XdndAware", "Xdnd"];
    const EXACT: &[&str] = &["RESOURCE_MANAGER", "SCREEN_RESOURCES", "RGB_DEFAULT_MAP", "_XKB_RULES_NAMES", "_XROOTPMAP_ID", "ESETROOT_PMAP_ID"];
    valid_name(name) && !PREFIXES.iter().any(|p| name.starts_with(p)) && !EXACT.contains(&name)
}

/// `SelectionRequest` reddi nedeni (içerik taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    Locked,
    /// Kök pencere, kendi penceremiz ya da yok.
    BadRequestor,
    BadProperty,
}

/// `SelectionRequest` ön denetimi (istemci yetkisinden önce). Sıra bağlayıcı: oturum kilidi her
/// istek anında **ilk** bakılır (kilitten önceki teklif de kilitliyken verilmez); sonra yanıtın
/// yazılacağı pencere (kök/kendi penceremiz değil) ve özellik adı ([`property_allowed`];
/// `None`: ad okunamadı). Gizli/hassas teklif buraya hiç gelmez: X11 sahipliği alınmaz
/// ([`decide_wayland`]).
pub fn check_request(locked: bool, requestor: u32, root: u32, own: u32, property: Option<&str>) -> Result<(), Reject> {
    if locked {
        return Err(Reject::Locked);
    }
    if requestor == 0 || requestor == root || requestor == own {
        return Err(Reject::BadRequestor);
    }
    if !property.is_some_and(property_allowed) {
        return Err(Reject::BadProperty);
    }
    Ok(())
}

/// Bir X11 istemcisinin (seçim isteyen ya da panoyu alan) köprüyü kullanma yetkisi.
///
/// `auto` (karar 2026-10-16): **yalnızca doğrulanmış konuk ajanı** — `window` (yanıtın yazılacağı
/// pencere ya da pano sahibinin penceresi) XRes'e göre bu kullanıcının konuk ajanı sürecine ait
/// olmalı. Odak temelli izin yok: `ConvertSelection`'da isteyen pencereyi istemci kendisi seçer;
/// odaksız bir istemci odaktaki pencereyi "isteyen" gösterip yanıtı o pencerenin özelliğinden
/// okuyabilirdi (X11'de her istemci her pencerenin özelliğini okur), odak isteği gönderene
/// bağlanamaz. Odaktaki X11 uygulamalarının panosunu Hyprland'in kendi XWM eşitlemesi taşır.
/// `on` kullanıcının açık seçimi: korumayı herkes için kaldırır (ayar açıklaması uyarır).
pub fn client_allowed(mode: BridgeMode, window_owned_by_agent: bool) -> bool {
    match mode {
        BridgeMode::On => true,
        BridgeMode::Auto => window_owned_by_agent,
        BridgeMode::Off => false,
    }
}

/// Ajanın penceresine yazılan yanıt özelliklerinin silinme takvimi: ajan okuyup silmezse
/// [`REPLY_TTL`] sonra köprü siler (içerik X11'de kalmasın). Aynı (pencere, özellik) yeniden
/// yazılırsa süre baştan başlar. En çok [`MAX_PENDING_REPLIES`] kayıt; fazlasında en eskisi
/// hemen silinmek üzere döner.
pub const REPLY_TTL: Duration = Duration::from_secs(2);
pub const MAX_PENDING_REPLIES: usize = 16;

#[derive(Debug, Default)]
pub struct ReplyCleanup {
    /// (pencere, özellik, silinme anı)
    pending: Vec<(u32, u32, Instant)>,
}

impl ReplyCleanup {
    /// Yazıldı; sınır aşılırsa hemen silinecek en eski kaydı döner.
    pub fn written(&mut self, window: u32, property: u32, now: Instant) -> Option<(u32, u32)> {
        self.pending.retain(|(w, p, _)| (*w, *p) != (window, property));
        self.pending.push((window, property, now + REPLY_TTL));
        (self.pending.len() > MAX_PENDING_REPLIES).then(|| {
            let (w, p, _) = self.pending.remove(0);
            (w, p)
        })
    }

    /// Süresi dolanları çıkarır ve döner.
    pub fn due(&mut self, now: Instant) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        self.pending.retain(|&(w, p, at)| {
            let keep = at > now;
            if !keep {
                out.push((w, p));
            }
            keep
        });
        out
    }

    /// Bir sonraki silinmeye kalan süre (yoksa `None`: sınırsız bekleme).
    pub fn next_in(&self, now: Instant) -> Option<Duration> {
        self.pending.iter().map(|(_, _, at)| at.saturating_duration_since(now)).min()
    }
}

/// Konuk ajanını arama anları (köprü başlangıcından itibaren). Ajanlar oturumla başlar; son
/// denemeden sonra tarama durur (boşta iş yok).
pub const AGENT_SCAN_AT: &[Duration] = &[
    Duration::ZERO,
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(40),
    Duration::from_secs(80),
    Duration::from_secs(160),
    Duration::from_secs(300),
];

/// Metin türleri, X11 → Wayland okumasında tercih sırasıyla (`TEXT` belirsiz: okunmaz).
const TEXT_READ: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING"];
/// Metnin sunulduğu adlar (her iki yönde; X11'de atom, Wayland'de MIME).
pub const TEXT_OFFER: &[&str] = &["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"];
/// X11'in seçim mekanizmasına ait hedefler (içerik türü değil).
/// Nuxut'un iç türleri (köprü ve süreç işaretleri); içerik değil, taşınmaz.
const NUXUT_PREFIX: &str = "application/x-nuxut-";
const X11_META: &[&str] =
    &["TARGETS", "TIMESTAMP", "MULTIPLE", "SAVE_TARGETS", "DELETE", "INSERT_SELECTION", "INSERT_PROPERTY", "LENGTH"];

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= MAX_NAME_LEN && !n.chars().any(char::is_control)
}

/// Ad hassas mı: parola yöneticisi ipucu ya da adında `secret` / `password` (framework ve
/// geçmiş filtresiyle aynı kural).
pub fn sensitive(name: &str) -> bool {
    nuxut::is_sensitive_mime(name)
}

/// Bir türün nasıl okunup yazılacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// Bayt aynen.
    Raw,
    /// UTF-8 metin; kaynak Latin-1 (`STRING`) ise UTF-8'e çevrilir.
    Utf8FromLatin1,
    /// Köprü işareti ([`BRIDGE_MIME`]): okunmaz, boş sunulur.
    Marker,
    /// Hedef Latin-1 (`STRING`); UTF-8 kaynak çevrilir, Latin-1 dışı karakter `?` olur.
    Latin1FromUtf8,
}

/// X11 → Wayland: hangi hedefler okunacak.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchPlan {
    /// Metin hedefi ve kodlaması (sonuç her zaman UTF-8).
    pub text: Option<(String, Encoding)>,
    /// Tek görsel türü (`image/png` önce).
    pub image: Option<String>,
    /// Diğer MIME benzeri türler (en çok [`MAX_EXTRA`]).
    pub extra: Vec<String>,
}

impl FetchPlan {
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.image.is_none() && self.extra.is_empty()
    }
}

/// X11 hedef listesinin kararı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XDecision {
    Fetch(FetchPlan),
    /// Hedeflerde köprü işareti var: Wayland'deki seçimimizin X11 yansıması.
    Ours,
    /// Hassas tür (parola yöneticisi ipucu vb.): taşınmaz, okunmaz.
    Sensitive,
    /// Liste sınırı aştı ya da bozuk ad.
    Malformed,
    /// Taşınacak tür yok.
    Nothing,
}

/// X11 sahibinin hedef adlarından okuma planı. İçerik okunmadan, yalnızca adlara bakar.
pub fn plan_fetch(targets: &[String]) -> XDecision {
    if targets.len() > MAX_TARGETS || targets.iter().any(|t| !valid_name(t)) {
        return XDecision::Malformed;
    }
    if targets.iter().any(|t| t == BRIDGE_MIME || t == nuxut::owner_mime()) {
        return XDecision::Ours;
    }
    if targets.iter().any(|t| sensitive(t)) {
        return XDecision::Sensitive;
    }
    let has = |n: &str| targets.iter().any(|t| t == n);
    let text = TEXT_READ.iter().find(|n| has(n)).map(|n| {
        let enc = if *n == "STRING" { Encoding::Utf8FromLatin1 } else { Encoding::Raw };
        ((*n).to_owned(), enc)
    });
    let image = if has("image/png") {
        Some("image/png".to_owned())
    } else {
        targets.iter().find(|t| t.starts_with("image/")).cloned()
    };
    let extra = targets
        .iter()
        .filter(|t| {
            t.contains('/')
                && !t.starts_with("image/")
                && !TEXT_OFFER.contains(&t.as_str())
                && !t.starts_with("text/plain")
                && !t.starts_with(NUXUT_PREFIX)
        })
        .take(MAX_EXTRA)
        .cloned()
        .collect();
    let plan = FetchPlan { text, image, extra };
    if plan.is_empty() { XDecision::Nothing } else { XDecision::Fetch(plan) }
}

/// Wayland → X11: bir X11 hedefinin karşılığı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// X11 hedef (atom) adı.
    pub name: String,
    /// Wayland teklifinden okunacak MIME.
    pub mime: String,
    pub encoding: Encoding,
}

/// Wayland teklifinin MIME listesinden X11'e sunulacak hedefler. Metin varsa X11'in metin
/// adlarının (UTF8_STRING, STRING, TEXT) hepsi eklenir. Boşsa sunulacak bir şey yok.
pub fn offer_targets(mimes: &[String]) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    let push = |out: &mut Vec<Target>, name: &str, mime: &str, encoding| {
        if !out.iter().any(|t| t.name == name) {
            out.push(Target { name: name.to_owned(), mime: mime.to_owned(), encoding });
        }
    };
    for m in mimes.iter().take(MAX_TARGETS) {
        if valid_name(m) && !X11_META.contains(&m.as_str()) && !m.starts_with(NUXUT_PREFIX) {
            push(&mut out, m, m, Encoding::Raw);
        }
    }
    let utf8 = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"].into_iter().find(|n| mimes.iter().any(|m| m == n));
    if let Some(src) = utf8 {
        for name in TEXT_OFFER {
            let enc = if *name == "STRING" { Encoding::Latin1FromUtf8 } else { Encoding::Raw };
            push(&mut out, name, src, enc);
        }
    }
    // İşaret: compositor bu X11 seçimini Wayland'e kendisi yansıtırsa geri gelen teklif tanınır.
    if !out.is_empty() {
        push(&mut out, BRIDGE_MIME, "", Encoding::Marker);
    }
    out
}

/// Bayt kodlamasını uygular (içerik loglanmaz). Geçersiz UTF-8 metin `None`.
pub fn convert(bytes: &[u8], encoding: Encoding) -> Option<Vec<u8>> {
    match encoding {
        Encoding::Raw => Some(bytes.to_vec()),
        Encoding::Marker => Some(Vec::new()),
        Encoding::Utf8FromLatin1 => Some(bytes.iter().map(|&b| char::from(b)).collect::<String>().into_bytes()),
        Encoding::Latin1FromUtf8 => {
            let s = std::str::from_utf8(bytes).ok()?;
            Some(s.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect())
        }
    }
}

/// Bir X11 penceresinin sahibi istemci (kaynak kimliğinin taban bitleri). Aynı istemcinin
/// pencereleri aynı tabanı paylaşır.
pub fn client_base(window: u32, resource_mask: u32) -> u32 {
    window & !resource_mask
}

/// Pano sahibi compositor'ın kendi X11 yöneticisi mi (`WM_S<n>` sahibiyle aynı istemci):
/// compositor Wayland seçimini X11'e kendisi yansıtmış; geri taşınmaz.
pub fn owned_by_compositor(owner: u32, wm_owner: Option<u32>, resource_mask: u32) -> bool {
    wm_owner.is_some_and(|wm| wm != 0 && client_base(owner, resource_mask) == client_base(wm, resource_mask))
}

/// Wayland teklifinin X11'e ne yapılacağı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WDecision {
    /// X11 panosunu al ve bu hedefleri tembel sun.
    Claim(Vec<Target>),
    /// Köprünün kendi X11 → Wayland seçimi ya da compositor'ın yansıtması: dokunma.
    Ignore,
    /// Taşınmaz (hassas/bozuk/boş): X11'de bizim eski seçimimiz varsa bırak.
    Release,
}

/// Son gerçek X11 sahibi değişimi (yansıtma sezgisi için).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XChange {
    pub at: Instant,
    /// O sahibin hedef adları (bilinmiyorsa boş).
    pub targets: Vec<String>,
}

/// Wayland teklifi kararı. `blocked`: kopyalama anında gizlilik kapısı kapalıydı (oturum kilitli
/// ya da hariç tutulan uygulama etkin). `offered_at`: teklifin ana thread'de alındığı an.
pub fn decide_wayland(
    mimes: &[String],
    sensitive_flag: bool,
    truncated: bool,
    blocked: bool,
    last_x: Option<&XChange>,
    offered_at: Instant,
) -> WDecision {
    if mimes.iter().any(|m| m == BRIDGE_MIME) {
        return WDecision::Ignore;
    }
    if blocked || sensitive_flag || truncated || mimes.len() > MAX_TARGETS || mimes.iter().any(|m| sensitive(m)) {
        return WDecision::Release;
    }
    if let Some(x) = last_x
        && offered_at.checked_duration_since(x.at).is_some_and(|d| d <= MIRROR_WINDOW)
        && !mimes.is_empty()
        && mimes.iter().all(|m| x.targets.iter().any(|t| t == m))
    {
        return WDecision::Ignore;
    }
    let targets = offer_targets(mimes);
    if targets.is_empty() { WDecision::Release } else { WDecision::Claim(targets) }
}

/// İçerik özeti (aynı içeriğin yeniden yansıtılmasını önlemek için; içerik saklanmaz).
pub fn digest(parts: &[(String, std::sync::Arc<[u8]>)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (m, b) in parts {
        m.hash(&mut h);
        b.hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn hypervisor_flag_and_agents() {
        assert!(cpuinfo_has_hypervisor("processor\t: 0\nflags\t\t: fpu vme hypervisor lahf_lm\n"));
        assert!(!cpuinfo_has_hypervisor("flags\t\t: fpu vme lahf_lm\nhypervisor_x : 1\n"));
        assert!(!cpuinfo_has_hypervisor(""));
        assert!(is_guest_agent("spice-vdagent\n"));
        assert!(is_guest_agent("VBoxClient"));
        assert!(is_guest_agent("vmtoolsd"));
        assert!(!is_guest_agent("spice-vdagentd"), "sistem daemon'u değil, oturum ajanı");
        assert!(!is_guest_agent("xclip"));
        // Başka kullanıcının aynı adlı süreci ajan sayılmaz.
        assert!(is_own_guest_agent("spice-vdagent\n", 1000, 1000));
        assert!(!is_own_guest_agent("spice-vdagent", 1001, 1000));
        assert!(!is_own_guest_agent("spice-vdagent", 0, 1000));
        assert!(!is_own_guest_agent("xclip", 1000, 1000));
    }

    #[test]
    fn only_own_xwayland_server() {
        assert!(is_own_xwayland("Xwayland\n", 1000, 1000));
        assert!(!is_own_xwayland("Xwayland", 0, 1000), "başka kullanıcı");
        for other in ["Xvnc", "Xorg", "Xephyr", "Xvfb", "xpra", ""] {
            assert!(!is_own_xwayland(other, 1000, 1000), "{other}");
        }
    }

    #[test]
    fn only_local_displays() {
        assert_eq!(local_display(":1"), Some(1));
        assert_eq!(local_display(":93.0"), Some(93));
        assert_eq!(local_display("unix:0"), Some(0));
        assert_eq!(local_display("unix:2.1"), Some(2));
        for bad in ["", ":", "localhost:10.0", "host:0", "tcp/host:0", ":1.", ":1.x", ":a", ":123456", "1", "::1:0", "/tmp/.X11-unix/X1", ":1 "] {
            assert_eq!(local_display(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn reply_property_cannot_target_wm_or_resource_properties() {
        for ok in ["XCLIP_OUT", "GDK_SELECTION", "_QT_SELECTION", "VDAGENT_SELECTION", "text/plain", "UTF8_STRING"] {
            assert!(property_allowed(ok), "{ok}");
        }
        for bad in ["WM_NAME", "WM_CLASS", "_NET_WM_NAME", "RESOURCE_MANAGER", "CUT_BUFFER0", "_XSETTINGS_S0", "_MOTIF_WM_HINTS", "XdndAware", "", "a\nb"] {
            assert!(!property_allowed(bad), "{bad:?}");
        }
    }

    #[test]
    fn request_precheck_order() {
        let (root, own, req) = (0x100, 0x200_0001, 0x400_0001);
        assert_eq!(check_request(false, req, root, own, Some("XCLIP_OUT")), Ok(()));
        // Kilit her şeyden önce (geçerli istek de reddedilir).
        assert_eq!(check_request(true, req, root, own, Some("XCLIP_OUT")), Err(Reject::Locked));
        assert_eq!(check_request(true, root, root, own, None), Err(Reject::Locked));
        // Kök/kendi/yok pencere.
        for bad in [root, own, 0] {
            assert_eq!(check_request(false, bad, root, own, Some("XCLIP_OUT")), Err(Reject::BadRequestor));
        }
        // Pencere yöneticisi / kaynak özellikleri ve okunamayan ad.
        assert_eq!(check_request(false, req, root, own, Some("WM_NAME")), Err(Reject::BadProperty));
        assert_eq!(check_request(false, req, root, own, Some("RESOURCE_MANAGER")), Err(Reject::BadProperty));
        assert_eq!(check_request(false, req, root, own, None), Err(Reject::BadProperty));
    }

    #[test]
    fn auto_serves_only_windows_owned_by_the_agent() {
        // auto: yalnız ajana ait pencere; odaktaki (ya da odağı taklit eden) istemci yetkisiz.
        assert!(client_allowed(BridgeMode::Auto, true));
        assert!(!client_allowed(BridgeMode::Auto, false));
        // on: kullanıcının açık seçimi; off: hiç.
        assert!(client_allowed(BridgeMode::On, false));
        assert!(!client_allowed(BridgeMode::Off, true));
    }

    #[test]
    fn unread_replies_are_deleted_after_ttl() {
        let t0 = Instant::now();
        let mut c = ReplyCleanup::default();
        assert_eq!(c.next_in(t0), None);
        assert_eq!(c.written(1, 10, t0), None);
        assert_eq!(c.next_in(t0), Some(REPLY_TTL));
        assert!(c.due(t0 + REPLY_TTL / 2).is_empty());
        // Yeniden yazım süreyi baştan başlatır (tek kayıt kalır).
        c.written(1, 10, t0 + REPLY_TTL / 2);
        assert!(c.due(t0 + REPLY_TTL).is_empty());
        assert_eq!(c.due(t0 + REPLY_TTL * 2), [(1, 10)]);
        assert_eq!(c.next_in(t0), None);
        // Sınır: en eskisi hemen silinmek üzere döner.
        for i in 0..MAX_PENDING_REPLIES as u32 {
            assert_eq!(c.written(2, i, t0), None);
        }
        assert_eq!(c.written(3, 0, t0), Some((2, 0)));
    }

    #[test]
    fn mode_parses_lowercase() {
        #[derive(serde::Deserialize)]
        struct T {
            m: BridgeMode,
        }
        let t: T = toml::from_str("m = \"on\"").unwrap();
        assert_eq!(t.m, BridgeMode::On);
        assert!(toml::from_str::<T>("m = \"yes\"").is_err());
        assert_eq!(BridgeMode::default(), BridgeMode::Auto);
    }

    #[test]
    fn fetch_plan_prefers_utf8_and_png() {
        let t = s(&["TARGETS", "TIMESTAMP", "STRING", "UTF8_STRING", "image/bmp", "image/png", "text/html", "text/uri-list"]);
        let XDecision::Fetch(p) = plan_fetch(&t) else { panic!() };
        assert_eq!(p.text, Some(("UTF8_STRING".into(), Encoding::Raw)));
        assert_eq!(p.image.as_deref(), Some("image/png"));
        assert_eq!(p.extra, s(&["text/html", "text/uri-list"]));
        let XDecision::Fetch(p) = plan_fetch(&s(&["STRING", "image/jpeg"])) else { panic!() };
        assert_eq!(p.text, Some(("STRING".into(), Encoding::Utf8FromLatin1)));
        assert_eq!(p.image.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn fetch_plan_guards() {
        assert_eq!(plan_fetch(&s(&["UTF8_STRING", BRIDGE_MIME])), XDecision::Ours);
        assert_eq!(plan_fetch(&s(&["UTF8_STRING", "x-kde-passwordManagerHint"])), XDecision::Sensitive);
        assert_eq!(plan_fetch(&s(&["UTF8_STRING", "application/x-Secret"])), XDecision::Sensitive);
        assert_eq!(plan_fetch(&s(&["TARGETS", "TIMESTAMP", "TEXT"])), XDecision::Nothing);
        let many: Vec<String> = (0..=MAX_TARGETS).map(|i| format!("a/{i}")).collect();
        assert_eq!(plan_fetch(&many), XDecision::Malformed);
        assert_eq!(plan_fetch(&s(&["UTF8_STRING", "bad\nname"])), XDecision::Malformed);
        assert_eq!(plan_fetch(&s(&["UTF8_STRING", &"x".repeat(MAX_NAME_LEN + 1)])), XDecision::Malformed);
        let XDecision::Fetch(p) = plan_fetch(&(0..10).map(|i| format!("app/x{i}")).collect::<Vec<_>>()) else { panic!() };
        assert_eq!(p.extra.len(), MAX_EXTRA);
    }

    #[test]
    fn offer_targets_add_x11_text_aliases() {
        let t = offer_targets(&s(&["text/plain;charset=utf-8", "image/png", "TARGETS"]));
        let names: Vec<_> = t.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["text/plain;charset=utf-8", "image/png", "text/plain", "UTF8_STRING", "STRING", "TEXT", BRIDGE_MIME]);
        let string = t.iter().find(|t| t.name == "STRING").unwrap();
        assert_eq!((string.mime.as_str(), string.encoding), ("text/plain;charset=utf-8", Encoding::Latin1FromUtf8));
        // Teklif STRING'i kendisi sunuyorsa aynen okunur.
        let t = offer_targets(&s(&["UTF8_STRING", "STRING"]));
        let string = t.iter().find(|t| t.name == "STRING").unwrap();
        assert_eq!((string.mime.as_str(), string.encoding), ("STRING", Encoding::Raw));
        assert!(offer_targets(&s(&["TARGETS"])).is_empty());
    }

    #[test]
    fn conversions() {
        assert_eq!(convert("şa€".as_bytes(), Encoding::Latin1FromUtf8).unwrap(), b"?a?");
        assert_eq!(convert("é".as_bytes(), Encoding::Latin1FromUtf8).unwrap(), [0xE9]);
        assert_eq!(convert(&[0xE9, b'x'], Encoding::Utf8FromLatin1).unwrap(), "éx".as_bytes());
        assert_eq!(convert(&[0xFF, 0xFE], Encoding::Latin1FromUtf8), None);
    }

    #[test]
    fn compositor_owner_by_client_base() {
        let mask = 0x001f_ffff;
        assert!(owned_by_compositor(0x0020_0005, Some(0x0020_0001), mask));
        assert!(!owned_by_compositor(0x0040_0005, Some(0x0020_0001), mask));
        assert!(!owned_by_compositor(0x0040_0005, None, mask));
        assert!(!owned_by_compositor(0x0000_0005, Some(0), mask));
    }

    #[test]
    fn wayland_decisions_prevent_loops() {
        let now = Instant::now();
        let text = s(&["text/plain;charset=utf-8", "UTF8_STRING"]);
        assert!(matches!(decide_wayland(&text, false, false, false, None, now), WDecision::Claim(_)));
        // Köprünün kendi X11 → Wayland seçimi.
        let mut ours = text.clone();
        ours.push(BRIDGE_MIME.into());
        assert_eq!(decide_wayland(&ours, false, false, false, None, now), WDecision::Ignore);
        // Hassas / kırpılmış: taşınmaz.
        assert_eq!(decide_wayland(&text, true, false, false, None, now), WDecision::Release);
        assert_eq!(decide_wayland(&text, false, true, false, None, now), WDecision::Release);
        assert_eq!(decide_wayland(&s(&["text/plain", "x-kde-passwordManagerHint"]), false, false, false, None, now), WDecision::Release);
        // Kopyalama anında kapı kapalı (kilit / hariç tutulan uygulama): taşınmaz, eski seçim bırakılır.
        assert_eq!(decide_wayland(&text, false, false, true, None, now), WDecision::Release);
        assert_eq!(decide_wayland(&ours, false, false, true, None, now), WDecision::Ignore);
        // Compositor'ın yansıtması: X11 sahibi az önce değişti ve türler onun hedefleri.
        let x = XChange { at: now, targets: s(&["UTF8_STRING", "text/plain;charset=utf-8", "STRING"]) };
        assert_eq!(decide_wayland(&text, false, false, false, Some(&x), now + Duration::from_millis(50)), WDecision::Ignore);
        // Süre geçti ya da tür kümesi farklı: gerçek Wayland kopyası.
        assert!(matches!(decide_wayland(&text, false, false, false, Some(&x), now + MIRROR_WINDOW * 2), WDecision::Claim(_)));
        assert!(matches!(decide_wayland(&s(&["text/html"]), false, false, false, Some(&x), now), WDecision::Claim(_)));
        assert_eq!(decide_wayland(&s(&["TARGETS"]), false, false, false, None, now), WDecision::Release);
    }

    #[test]
    fn digest_distinguishes_content() {
        let a = vec![("text/plain".to_owned(), std::sync::Arc::<[u8]>::from(&b"a"[..]))];
        let b = vec![("text/plain".to_owned(), std::sync::Arc::<[u8]>::from(&b"b"[..]))];
        assert_eq!(digest(&a), digest(&a.clone()));
        assert_ne!(digest(&a), digest(&b));
    }
}

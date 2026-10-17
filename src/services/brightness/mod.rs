//! Ekran parlaklığı: dahili panel (arka ışık) ve harici monitörler (DDC/CI).
//!
//! **Dahili.** `/sys/class/backlight` okunur; yazma systemd-logind `Session.SetBrightness`
//! ile yapılır — root ya da udev kuralı gerekmez, logind oturum sahibine izin verir. Değişim
//! (klavye tuşu, `brightnessctl`, bizim isteğimiz) kernel'in `actual_brightness` üzerindeki
//! `sysfs_notify`'ı ile `poll(POLLPRI)` beklenerek fark edilir: polling yok, ayrı bir
//! izleyici thread'i olay gelene kadar uyur.
//!
//! **Harici.** DDC/CI doğrudan `/dev/i2c-N` ile ([`ddc`]): bağlayıcı → i2c hattı
//! `/sys/class/drm` üzerinden bulunur. Monitörler yavaştır (mesaj başına 40–50 ms), bu yüzden
//! ayrı bir thread'de çalışır ve istekler birleştirilir: kaydırıcı sürüklenirken kuyrukta
//! bekleyen ara değerler atılır, monitör başına yalnızca sonuncusu yazılır. Erişim için
//! kullanıcının i2c aygıtlarına izni olmalı (ddcutil paketinin udev kuralı `uaccess` verir);
//! yoksa harici monitörler listede görünmez.
//!
//! Değer değişince [`BrightnessEvent::Changed`] (OSD için) yayınlanır; aynı değer tekrar
//! yayınlanmaz.

mod ddc;
mod sysfs;

use std::collections::HashMap;
use std::os::fd::{AsFd, OwnedFd};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use nuxut::i18n::t;

pub use ddc::{DdcError, VcpValue, get_vcp_request, parse_vcp_reply, set_vcp_request};

use ddc::{DdcBus, VCP_BRIGHTNESS};
use sysfs::{Backlight, Connector};

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION_AUTO: &str = "/org/freedesktop/login1/session/auto";
const SESSION_IFACE: &str = "org.freedesktop.login1.Session";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayKind {
    /// Dizüstü paneli (arka ışık).
    Internal,
    /// DDC/CI destekli harici monitör.
    External,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// Kararlı kimlik: `backlight:<aygıt>` ya da `ddc:<çıktı>`.
    pub id: String,
    pub kind: DisplayKind,
    /// Harici monitörde Wayland çıktı adı (`DP-1`); dahilide `None`.
    pub output: Option<String>,
    /// 0..=100.
    pub percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrightnessEvent {
    /// Ekran listesi (başlangıçta ve [`BrightnessService::rescan`] sonrası).
    Displays(Vec<Display>),
    /// Bir ekranın parlaklığı değişti (OSD).
    Changed(Display),
    Failed {
        id: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct BrightnessOptions {
    /// sysfs kökü (testler için).
    pub sysfs_root: PathBuf,
    /// i2c aygıtlarının dizini.
    pub dev_root: PathBuf,
    /// logind'in bulunduğu bus adresi; `None`: sistem bus'ı.
    pub bus_address: Option<String>,
    pub call_timeout: Duration,
    /// Dahili panelin inebileceği en düşük yüzde (0 bazı panellerde ekranı tamamen karartır).
    pub internal_min_percent: f64,
    /// Harici monitörleri DDC ile ara.
    pub ddc: bool,
}

impl Default for BrightnessOptions {
    fn default() -> Self {
        Self {
            sysfs_root: PathBuf::from("/sys"),
            dev_root: PathBuf::from("/dev"),
            bus_address: None,
            call_timeout: Duration::from_secs(2),
            internal_min_percent: 1.0,
            ddc: true,
        }
    }
}

#[derive(Debug)]
enum Msg {
    Set {
        id: String,
        percent: f64,
    },
    Adjust {
        id: String,
        delta: f64,
    },
    Rescan,
    /// Arka ışık değişti (izleyici thread'den).
    Backlight,
    Ddc(DdcReport),
    Stop,
}

#[derive(Debug)]
enum DdcMsg {
    Probe(Vec<Connector>),
    Set { output: String, value: u16 },
}

#[derive(Debug)]
enum DdcReport {
    Probed(Vec<(String, VcpValue)>),
    Value { output: String, value: VcpValue },
    Failed { output: String, message: String },
}

/// Çalışan parlaklık servisi. Düşürülünce izleyici ve DDC thread'leri durur, join edilir.
#[derive(Debug)]
pub struct BrightnessService {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl BrightnessService {
    pub fn spawn<F>(options: BrightnessOptions, sink: F) -> std::io::Result<Self>
    where
        F: Fn(BrightnessEvent) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let self_tx = tx.clone();
        let thread = std::thread::Builder::new()
            .name("nuxut-brightness".into())
            .spawn(move || Worker::new(options, Box::new(sink), self_tx).run(&rx))?;
        Ok(Self { tx, thread: Some(thread) })
    }

    /// Parlaklığı yüzde olarak ayarlar (0..=100, sınırlanır).
    pub fn set(&self, id: &str, percent: f64) {
        let _ = self.tx.send(Msg::Set { id: id.to_owned(), percent });
    }

    /// Göreli değişim (yüzde puanı, ör. +5 / -5).
    pub fn adjust(&self, id: &str, delta: f64) {
        let _ = self.tx.send(Msg::Adjust { id: id.to_owned(), delta });
    }

    /// Ekranları yeniden tara (monitör takılıp çıkarılınca).
    pub fn rescan(&self) {
        let _ = self.tx.send(Msg::Rescan);
    }
}

impl Drop for BrightnessService {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take()
            && t.join().is_err()
        {
            tracing::error!("parlaklık thread'i panikle sonlandı");
        }
    }
}

fn clamp_percent(p: f64, min: f64) -> f64 {
    if p.is_finite() { p.clamp(min, 100.0) } else { min }
}

/// Ham değer → yüzde.
fn to_percent(raw: u32, max: u32) -> f64 {
    if max == 0 { 0.0 } else { f64::from(raw.min(max)) * 100.0 / f64::from(max) }
}

/// Yüzde → ham değer (en yakın).
fn to_raw(percent: f64, max: u32) -> u32 {
    ((percent.clamp(0.0, 100.0) / 100.0) * f64::from(max)).round() as u32
}

struct External {
    output: String,
    value: VcpValue,
}

struct Worker {
    options: BrightnessOptions,
    sink: Box<dyn Fn(BrightnessEvent) + Send>,
    self_tx: Sender<Msg>,
    backlight: Option<Backlight>,
    internal_raw: Option<u32>,
    externals: Vec<External>,
    /// Son yayınlanan yüzde (aynısı tekrar yayınlanmaz).
    emitted: HashMap<String, f64>,
    logind: Option<zbus::blocking::Connection>,
    watcher: Option<Watcher>,
    ddc: Option<(Sender<DdcMsg>, JoinHandle<()>)>,
}

impl Worker {
    fn new(options: BrightnessOptions, sink: Box<dyn Fn(BrightnessEvent) + Send>, self_tx: Sender<Msg>) -> Self {
        Self {
            options,
            sink,
            self_tx,
            backlight: None,
            internal_raw: None,
            externals: Vec::new(),
            emitted: HashMap::new(),
            logind: None,
            watcher: None,
            ddc: None,
        }
    }

    fn run(mut self, rx: &Receiver<Msg>) {
        self.scan();
        while let Ok(first) = rx.recv() {
            // Kuyrukta bekleyenleri birleştir: ekran başına son hedef, tek yeniden okuma.
            let mut batch = vec![first];
            batch.extend(rx.try_iter());
            let mut targets: Vec<(String, f64)> = Vec::new();
            let mut reread = false;
            let mut rescan = false;
            for msg in batch {
                match msg {
                    Msg::Stop => {
                        self.shutdown();
                        return;
                    }
                    Msg::Set { id, percent } => {
                        targets.retain(|(i, _)| *i != id);
                        targets.push((id, percent));
                    }
                    Msg::Adjust { id, delta } => {
                        let base =
                            targets.iter().find(|(i, _)| *i == id).map(|(_, p)| *p).or_else(|| self.percent_of(&id));
                        if let Some(base) = base {
                            targets.retain(|(i, _)| *i != id);
                            targets.push((id, base + delta));
                        }
                    }
                    Msg::Rescan => rescan = true,
                    Msg::Backlight => reread = true,
                    Msg::Ddc(report) => self.ddc_report(report),
                }
            }
            if rescan {
                self.scan();
            }
            for (id, percent) in targets {
                self.apply(&id, percent);
            }
            if reread {
                self.reread_internal();
            }
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(w) = self.watcher.take() {
            w.stop();
        }
        if let Some((tx, thread)) = self.ddc.take() {
            drop(tx);
            let _ = thread.join();
        }
    }

    fn internal_id(b: &Backlight) -> String {
        format!("backlight:{}", b.name)
    }

    fn percent_of(&self, id: &str) -> Option<f64> {
        if let Some(b) = &self.backlight
            && id == Self::internal_id(b)
        {
            return self.internal_raw.map(|r| to_percent(r, b.max));
        }
        let output = id.strip_prefix("ddc:")?;
        self.externals
            .iter()
            .find(|e| e.output == output)
            .map(|e| to_percent(u32::from(e.value.current), u32::from(e.value.max)))
    }

    fn displays(&self) -> Vec<Display> {
        let mut out = Vec::new();
        if let (Some(b), Some(raw)) = (&self.backlight, self.internal_raw) {
            out.push(Display {
                id: Self::internal_id(b),
                kind: DisplayKind::Internal,
                output: None,
                percent: to_percent(raw, b.max),
            });
        }
        for e in &self.externals {
            out.push(Display {
                id: format!("ddc:{}", e.output),
                kind: DisplayKind::External,
                output: Some(e.output.clone()),
                percent: to_percent(u32::from(e.value.current), u32::from(e.value.max)),
            });
        }
        out
    }

    fn publish_all(&mut self) {
        let displays = self.displays();
        for d in &displays {
            self.emitted.insert(d.id.clone(), d.percent);
        }
        (self.sink)(BrightnessEvent::Displays(displays));
    }

    fn publish_changed(&mut self, id: &str) {
        let Some(d) = self.displays().into_iter().find(|d| d.id == id) else {
            return;
        };
        if self.emitted.get(id).is_some_and(|p| (p - d.percent).abs() < 0.01) {
            return;
        }
        self.emitted.insert(d.id.clone(), d.percent);
        (self.sink)(BrightnessEvent::Changed(d));
    }

    fn fail(&self, id: &str, message: String) {
        tracing::warn!(id, "parlaklık: {message}");
        (self.sink)(BrightnessEvent::Failed { id: id.to_owned(), message });
    }

    fn scan(&mut self) {
        if let Some(w) = self.watcher.take() {
            w.stop();
        }
        self.backlight = sysfs::backlights(&self.options.sysfs_root).into_iter().next();
        self.internal_raw = self.backlight.as_ref().and_then(sysfs::current);
        if let Some(b) = &self.backlight {
            match Watcher::start(b, self.self_tx.clone()) {
                Ok(w) => self.watcher = Some(w),
                Err(e) => tracing::warn!("arka ışık izlenemiyor ({e}); dış değişiklikler görünmez"),
            }
        }
        if self.options.ddc {
            let connectors = sysfs::connectors(&self.options.sysfs_root);
            self.externals.retain(|e| connectors.iter().any(|c| c.name == e.output));
            if !connectors.is_empty() {
                self.ddc_sender().map(|tx| tx.send(DdcMsg::Probe(connectors)));
            }
        }
        self.publish_all();
    }

    fn ddc_sender(&mut self) -> Option<&Sender<DdcMsg>> {
        if self.ddc.is_none() {
            let (tx, rx) = mpsc::channel();
            let dev_root = self.options.dev_root.clone();
            let report = self.self_tx.clone();
            match std::thread::Builder::new()
                .name("nuxut-ddc".into())
                .spawn(move || ddc_worker(&dev_root, &rx, &report))
            {
                Ok(thread) => self.ddc = Some((tx, thread)),
                Err(e) => tracing::warn!("DDC thread'i başlatılamadı: {e}"),
            }
        }
        self.ddc.as_ref().map(|(tx, _)| tx)
    }

    fn ddc_report(&mut self, report: DdcReport) {
        match report {
            DdcReport::Probed(found) => {
                self.externals = found.into_iter().map(|(output, value)| External { output, value }).collect();
                self.publish_all();
            }
            DdcReport::Value { output, value } => {
                if let Some(e) = self.externals.iter_mut().find(|e| e.output == output) {
                    e.value = value;
                    self.publish_changed(&format!("ddc:{output}"));
                }
            }
            DdcReport::Failed { output, message } => self.fail(&format!("ddc:{output}"), message),
        }
    }

    fn apply(&mut self, id: &str, percent: f64) {
        if let Some(b) = self.backlight.clone()
            && id == Self::internal_id(&b)
        {
            let raw = to_raw(clamp_percent(percent, self.options.internal_min_percent), b.max);
            if self.internal_raw == Some(raw) {
                return;
            }
            match self.set_backlight(&b.name, raw) {
                Ok(()) => {
                    self.internal_raw = Some(raw);
                    self.publish_changed(id);
                }
                Err(e) => {
                    self.logind = None;
                    self.fail(id, t!("shell-brightness-error-logind-set", detail = e.to_string()));
                }
            }
            return;
        }
        let Some(output) = id.strip_prefix("ddc:") else {
            self.fail(id, t!("shell-brightness-error-unknown-display"));
            return;
        };
        let Some(ext) = self.externals.iter_mut().find(|e| e.output == output) else {
            self.fail(id, t!("shell-brightness-error-unknown-display"));
            return;
        };
        let value = to_raw(clamp_percent(percent, 0.0), u32::from(ext.value.max)) as u16;
        if value == ext.value.current {
            return;
        }
        // İyimser: OSD hemen tepki versin; DDC yazımı arka planda.
        ext.value.current = value;
        let output = output.to_owned();
        self.publish_changed(id);
        self.ddc_sender().map(|tx| tx.send(DdcMsg::Set { output, value }));
    }

    fn set_backlight(&mut self, name: &str, raw: u32) -> zbus::Result<()> {
        if self.logind.is_none() {
            let builder = match &self.options.bus_address {
                Some(a) => zbus::blocking::connection::Builder::address(a.as_str())?,
                None => zbus::blocking::connection::Builder::system()?,
            };
            self.logind = Some(builder.method_timeout(self.options.call_timeout).build()?);
        }
        let Some(conn) = &self.logind else { unreachable!() };
        conn.call_method(Some(LOGIN1), SESSION_AUTO, Some(SESSION_IFACE), "SetBrightness", &("backlight", name, raw))
            .map(drop)
    }

    fn reread_internal(&mut self) {
        let Some(b) = self.backlight.clone() else {
            return;
        };
        let raw = sysfs::current(&b);
        if raw.is_some() && raw != self.internal_raw {
            self.internal_raw = raw;
            self.publish_changed(&Self::internal_id(&b));
        }
    }
}

// ---------------------------------------------------------------------------------------
// Arka ışık izleyici

/// `actual_brightness` üzerinde `poll(POLLPRI)`; durdurmak için eventfd.
struct Watcher {
    stop: OwnedFd,
    thread: JoinHandle<()>,
}

impl Watcher {
    fn start(b: &Backlight, tx: Sender<Msg>) -> std::io::Result<Self> {
        use rustix::event::{EventfdFlags, eventfd};
        let path = ["actual_brightness", "brightness"].iter().map(|f| b.dir.join(f)).find(|p| p.exists());
        let Some(path) = path else {
            return Err(std::io::ErrorKind::NotFound.into());
        };
        let file = std::fs::File::open(&path)?;
        let stop = eventfd(0, EventfdFlags::CLOEXEC)?;
        let stop_rx = stop.try_clone()?;
        let thread =
            std::thread::Builder::new().name("nuxut-backlight".into()).spawn(move || watch(&file, &stop_rx, &tx))?;
        Ok(Self { stop, thread })
    }

    fn stop(self) {
        let _ = rustix::io::write(&self.stop, &1u64.to_ne_bytes());
        let _ = self.thread.join();
    }
}

fn watch(file: &std::fs::File, stop: &OwnedFd, tx: &Sender<Msg>) {
    use rustix::event::{PollFd, PollFlags, poll};
    let mut buf = [0u8; 32];
    // kernfs: okuma olay sayacını sıfırlar; ilk poll'dan önce bir kez oku.
    let _ = rustix::io::pread(file, &mut buf, 0);
    loop {
        let mut fds = [PollFd::new(file, PollFlags::PRI | PollFlags::ERR), PollFd::new(stop, PollFlags::IN)];
        if let Err(e) = poll(&mut fds, None) {
            if e == rustix::io::Errno::INTR {
                continue;
            }
            tracing::warn!("arka ışık poll hatası: {e}");
            return;
        }
        if !fds[1].revents().is_empty() {
            return;
        }
        let revents = fds[0].revents();
        if revents.contains(PollFlags::NVAL) {
            return;
        }
        if revents.intersects(PollFlags::PRI | PollFlags::ERR) {
            let _ = rustix::io::pread(file.as_fd(), &mut buf, 0);
            if tx.send(Msg::Backlight).is_err() {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// DDC thread'i

fn ddc_worker(dev_root: &std::path::Path, rx: &Receiver<DdcMsg>, report: &Sender<Msg>) {
    let mut buses: HashMap<String, (DdcBus, VcpValue)> = HashMap::new();
    let send = |r: DdcReport| report.send(Msg::Ddc(r)).is_ok();
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        batch.extend(rx.try_iter());
        // Son tarama + monitör başına son değer.
        let mut probe = None;
        let mut sets: Vec<(String, u16)> = Vec::new();
        for m in batch {
            match m {
                DdcMsg::Probe(c) => probe = Some(c),
                DdcMsg::Set { output, value } => {
                    sets.retain(|(o, _)| *o != output);
                    sets.push((output, value));
                }
            }
        }
        if let Some(connectors) = probe {
            buses.clear();
            let mut found = Vec::new();
            for c in connectors {
                let path = dev_root.join(format!("i2c-{}", c.i2c));
                let probed = DdcBus::open(&path).and_then(|mut bus| bus.get_vcp(VCP_BRIGHTNESS).map(|v| (bus, v)));
                match probed {
                    Ok((bus, v)) => {
                        found.push((c.name.clone(), v));
                        buses.insert(c.name, (bus, v));
                    }
                    Err(e) => tracing::debug!(output = c.name, "DDC yok: {e}"),
                }
            }
            if !send(DdcReport::Probed(found)) {
                return;
            }
        }
        for (output, value) in sets {
            let Some((bus, v)) = buses.get_mut(&output) else {
                continue;
            };
            let r = match bus.set_vcp(VCP_BRIGHTNESS, value) {
                Ok(()) => {
                    v.current = value;
                    DdcReport::Value { output, value: *v }
                }
                Err(e) => DdcReport::Failed { output, message: ddc_message(&e) },
            };
            if !send(r) {
                return;
            }
        }
    }
}

/// Kartın hata satırı (`DdcError`'un Display'i log için kalır).
fn ddc_message(e: &DdcError) -> String {
    match e {
        DdcError::BadReply => t!("shell-brightness-error-ddc-bad-reply"),
        DdcError::Checksum => t!("shell-brightness-error-ddc-checksum"),
        DdcError::Unsupported => t!("shell-brightness-error-ddc-unsupported"),
        DdcError::Io(detail) => t!("shell-brightness-error-ddc-io", detail = detail.as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_roundtrip() {
        assert_eq!(to_raw(50.0, 96000), 48000);
        assert_eq!(to_percent(48000, 96000), 50.0);
        assert_eq!(to_raw(150.0, 15), 15);
        assert_eq!(to_raw(-3.0, 15), 0);
        assert_eq!(clamp_percent(f64::NAN, 1.0), 1.0);
        assert_eq!(clamp_percent(0.0, 1.0), 1.0);
    }
}

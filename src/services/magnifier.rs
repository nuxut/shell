//! Büyüteç (t124, erişilebilirlik): Hyprland `cursor:zoom_factor` ile imleç çevresinde ekranı
//! büyütür. Tasarım ve kararlar: `docs/design/magnifier.md`.
//!
//! UI'dan bağımsızdır: saf durum makinesi ([`Magnifier`]), Hyprland komut metinleri
//! ([`write_command`], [`READ_COMMAND`]) ve [`HyprControl`] üzerinden tek adımlar ([`run`],
//! [`release`]). Çağıran (`app/magnifier.rs`) hepsini ayrı bir thread'de yürütür; Hyprland
//! isteği UI'ı bekletmez.
//!
//! **Sahiplik kuralı** (kullanıcının elle ayarladığı büyütmeyi ezmemek için): kabuk yalnızca
//! KENDİ yazdığı değeri geri alır. [`Magnifier::managed`] kabuğun en son yazdığı 1'den büyük
//! çarpandır. Çıkışta (ve çökme sonrası açılışta, [`Marker`] ile) Hyprland'in canlı değeri bu
//! değere eşitse 1'e döner; farklıysa (kullanıcı `hyprctl` ya da kendi bind'iyle değiştirmiş, ya
//! da Hyprland ayarı yeniden yüklenmiş) dokunulmaz ve sahiplik bırakılır.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use nuxut::i18n::t;
use serde::{Deserialize, Serialize};

use crate::schema::SettingSchema;
use crate::services::hypr_config::control::HyprControl;

/// Hyprland seçeneği.
pub const OPTION: &str = "cursor:zoom_factor";
/// Canlı değer sorgusu.
pub const READ_COMMAND: &str = "j/getoption cursor:zoom_factor";
/// Ayar dili sorgusu (`configProvider`).
pub const PROVIDER_COMMAND: &str = "j/status";
/// Büyütmesiz değer (Hyprland'in alt sınırı).
pub const MIN_FACTOR: f64 = 1.0;
/// Hyprland'in kabul ettiği üst sınır (v0.56.2 `hyprctl descriptions`: min 1, max 10).
pub const HYPR_MAX_FACTOR: f64 = 10.0;
/// Çarpımsal adım sınırı (`[magnifier] step`).
pub const STEP_RANGE: std::ops::RangeInclusive<f32> = 1.05..=2.0;
/// Tavan sınırı (`[magnifier] max`).
pub const MAX_RANGE: std::ops::RangeInclusive<f32> = 1.5..=10.0;
/// Açılış çarpanı sınırı (`[magnifier] factor`; ayrıca `max`'ı aşmaz).
pub const FACTOR_RANGE: std::ops::RangeInclusive<f32> = 1.1..=10.0;
/// İki çarpan bu kadar yakınsa aynı sayılır (Hyprland `float` 6 basamak yazar; biz 3 yazarız).
const EPS: f64 = 1e-3;

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

/// 3 ondalığa yuvarlar (komut metni ve karşılaştırma aynı değeri görsün).
fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

// ---------------------------------------------------------------------------------------
// Ayarlar

/// `[magnifier]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct MagnifierConfig {
    /// `in`/`out` çarpımsal adımı ([`STEP_RANGE`]).
    pub step: f32,
    /// En büyük çarpan ([`MAX_RANGE`]); `in` ve `set` bunu aşmaz.
    pub max: f32,
    /// Kapalıyken açınca (kutucuk, `msg magnifier on|toggle`) kullanılan çarpan; oturumda son
    /// kullanılan çarpan varsa o tercih edilir ([`FACTOR_RANGE`], `max`'ı aşmaz).
    pub factor: f32,
}

impl Default for MagnifierConfig {
    fn default() -> Self {
        Self { step: 1.25, max: 8.0, factor: 2.0 }
    }
}

impl MagnifierConfig {
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(
            SettingSchema::float("magnifier.step", t!("shell-config-magnifier-step-label"), d.step, *STEP_RANGE.start(), *STEP_RANGE.end())
                .step(0.05)
                .describe(t!("shell-config-magnifier-step-describe")),
        );
        out.push(
            SettingSchema::float("magnifier.max", t!("shell-config-magnifier-max-label"), d.max, *MAX_RANGE.start(), *MAX_RANGE.end())
                .step(0.5),
        );
        out.push(
            SettingSchema::float("magnifier.factor", t!("shell-config-magnifier-factor-label"), d.factor, *FACTOR_RANGE.start(), *FACTOR_RANGE.end())
                .step(0.25)
                .describe(t!("shell-config-magnifier-factor-describe")),
        );
    }

    /// Değerleri sınırlara çeker; her düzeltme için uyarı. İdempotent.
    pub fn validate(&mut self, w: &mut Vec<String>) {
        let d = Self::default();
        let mut fix = |value: &mut f32, key: &str, range: std::ops::RangeInclusive<f32>, default: f32| {
            if !value.is_finite() || !range.contains(value) {
                let fixed = if value.is_finite() { value.clamp(*range.start(), *range.end()) } else { default };
                w.push(t!(
                    "shell-config-magnifier-out-of-range",
                    key = key,
                    value = format!("{value}"),
                    fixed = format!("{fixed}"),
                    range = format!("{}..={}", range.start(), range.end())
                ));
                *value = fixed;
            }
        };
        fix(&mut self.step, "magnifier.step", STEP_RANGE, d.step);
        fix(&mut self.max, "magnifier.max", MAX_RANGE, d.max);
        fix(&mut self.factor, "magnifier.factor", FACTOR_RANGE, d.factor);
        // `factor > max` uyarı değil: anahtarlar tek tek geçerlidir (Ayarlar birini değiştirirken
        // öteki uyarı vermesin); kullanımda `max`'a kırpılır ([`Magnifier::set_config`]).
    }
}

// ---------------------------------------------------------------------------------------
// Komutlar

/// Binde bir çözünürlüklü çarpan: karşılaştırılabilir (`Eq`), widget eylemlerinde taşınır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Factor(u32);

impl Factor {
    /// Sonlu olmayan değer 1 sayılır; 0..=1000 dışı kırpılır.
    pub fn new(v: f64) -> Self {
        let v = if v.is_finite() { v.clamp(0.0, 1000.0) } else { MIN_FACTOR };
        Self((v * 1000.0).round() as u32)
    }

    pub fn get(self) -> f64 {
        f64::from(self.0) / 1000.0
    }
}

/// Büyüteç eylemi (`msg magnifier …`, kutucuk, kısayollar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Canlı değer × adım (tavana kırpılır).
    In,
    /// Canlı değer ÷ adım (1'e kırpılır).
    Out,
    /// 1 (büyütme yok).
    Reset,
    /// Çarpan; ayrıştırıcı [`MIN_FACTOR`]..=[`HYPR_MAX_FACTOR`] dışını reddeder, tavanı
    /// ([`MagnifierConfig::max`]) yürütücü denetler.
    Set(Factor),
    /// Açıksa kapat, kapalıysa son kullanılan (yoksa `[magnifier] factor`) çarpanla aç.
    Toggle,
    On,
    Off,
}

/// Komut satırındaki çarpan: ondalık nokta, işaretsiz, sonlu, [`MIN_FACTOR`]..=[`HYPR_MAX_FACTOR`].
pub fn parse_factor(word: &str) -> Option<f64> {
    if word.is_empty() || word.len() > 8 || !word.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let v: f64 = word.parse().ok()?;
    (v.is_finite() && (MIN_FACTOR..=HYPR_MAX_FACTOR).contains(&v)).then_some(v)
}

/// Komutun reddi.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    /// `set` tavanın üstünde.
    AboveMax { value: f64, max: f64 },
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::AboveMax { value, max } => {
                write!(f, "{value} tavanın üstünde ({MIN_FACTOR}-{max}; [magnifier] max)")
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Durum makinesi

/// Kutucuğun ve `msg magnifier status`un gördüğü durum.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MagnifierView {
    /// Hyprland'e komut gönderilebiliyor ve cevap veriyor.
    pub available: bool,
    /// Canlı çarpan (bilinmiyorsa 1).
    pub factor: f64,
    /// Kaydırıcının üst sınırı (`[magnifier] max`).
    pub max: f64,
}

impl MagnifierView {
    pub fn active(&self) -> bool {
        self.factor > MIN_FACTOR + EPS
    }
}

/// `msg magnifier status` (tek satır JSON, sözleşme magnifier-v1; alanlar yalnızca eklenir).
#[derive(Debug, Serialize)]
pub struct Status {
    pub v: u32,
    pub available: bool,
    /// Canlı çarpan; `available = false` iken `null`.
    pub factor: Option<f64>,
    pub active: bool,
    /// Canlı değer kabuğun yazdığı değer mi (çıkışta 1'e dönecek mi).
    pub managed: bool,
    pub step: f64,
    pub max: f64,
    /// `available = false` nedeni (tanı; çevrilmez).
    pub reason: Option<String>,
}

impl Status {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"v\":1,\"available\":false}".into())
    }
}

/// Saf durum makinesi; Hyprland'e dokunmaz.
#[derive(Debug, Clone, PartialEq)]
pub struct Magnifier {
    step: f64,
    max: f64,
    factor: f64,
    /// Kabuğun en son yazdığı (> 1) değer: yalnızca bu geri alınır.
    managed: Option<f64>,
    /// Bu oturumda son kullanılan (> 1) çarpan: `on`/`toggle` onu geri getirir.
    last_on: Option<f64>,
}

impl Magnifier {
    pub fn new(config: &MagnifierConfig) -> Self {
        let mut m = Self { step: 1.25, max: 8.0, factor: 2.0, managed: None, last_on: None };
        m.set_config(config);
        m
    }

    /// Ayar değişti (doğrulanmış değerler; yine de sınırlar zorlanır).
    pub fn set_config(&mut self, config: &MagnifierConfig) {
        self.step = f64::from(config.step).clamp(f64::from(*STEP_RANGE.start()), f64::from(*STEP_RANGE.end()));
        self.max = f64::from(config.max).clamp(f64::from(*MAX_RANGE.start()), HYPR_MAX_FACTOR);
        self.factor = f64::from(config.factor).clamp(MIN_FACTOR, self.max);
    }

    pub fn managed(&self) -> Option<f64> {
        self.managed
    }

    pub fn max(&self) -> f64 {
        self.max
    }

    pub fn step(&self) -> f64 {
        self.step
    }

    /// Canlı değer okundu: kabuğun yazdığı değerden farklıysa sahiplik bırakılır (kullanıcı ya
    /// da ayar yeniden yüklemesi değiştirdi; artık kabuk geri almaz).
    pub fn observe(&mut self, live: f64) {
        if self.managed.is_some_and(|m| !same(m, live)) {
            self.managed = None;
        }
    }

    /// Komutun hedef çarpanı (canlı değerden). Sonuç [`MIN_FACTOR`]..=`max` içinde, 3 ondalık.
    pub fn target(&self, cmd: Command, live: f64) -> Result<f64, PlanError> {
        let live = live.max(MIN_FACTOR);
        let t = match cmd {
            Command::In => (live * self.step).min(self.max).max(live.min(self.max)),
            Command::Out => {
                let t = live / self.step;
                // Adımın küçük bir kesrine kalan büyütme gözle görülmez: 1'e yapışır.
                if t < MIN_FACTOR + 0.02 { MIN_FACTOR } else { t }
            }
            Command::Reset | Command::Off => MIN_FACTOR,
            Command::Set(v) => {
                let v = v.get();
                if v > self.max + EPS {
                    return Err(PlanError::AboveMax { value: v, max: self.max });
                }
                v
            }
            Command::On => self.on_factor(),
            Command::Toggle => {
                if live > MIN_FACTOR + EPS {
                    MIN_FACTOR
                } else {
                    self.on_factor()
                }
            }
        };
        Ok(round3(t.clamp(MIN_FACTOR, self.max)))
    }

    fn on_factor(&self) -> f64 {
        self.last_on.unwrap_or(self.factor).clamp(MIN_FACTOR, self.max)
    }

    /// Kabuk `factor`'ü Hyprland'e yazdı.
    pub fn wrote(&mut self, factor: f64) {
        if factor > MIN_FACTOR + EPS {
            self.managed = Some(factor);
            self.last_on = Some(factor);
        } else {
            self.managed = None;
        }
    }

    /// Kabuk yazmadı (canlı değer zaten hedefteydi). Hedef 1 ise sahiplik biter; değilse
    /// sahiplik değişmez (kullanıcının kendi değerini kabuk sahiplenmez).
    pub fn unchanged(&mut self, factor: f64) {
        if factor <= MIN_FACTOR + EPS {
            self.managed = None;
        } else if self.managed.is_some() {
            self.last_on = Some(factor);
        }
    }

    /// Hyprland ayarı yeniden yüklendi (`configreloaded`): canlı değer 1'e döndüyse (ayar dosyası
    /// büyütme vermiyor) kabuğun büyütmesi yeniden yazılır (`Some(m)`). Canlı değer kabuğunkiyse
    /// iş yok; başka bir değerse (kullanıcının ayarı ya da elle değişiklik) sahiplik bırakılır.
    pub fn reload_target(&mut self, live: f64) -> Option<f64> {
        let m = self.managed?;
        if same(m, live) {
            return None;
        }
        if same(live, MIN_FACTOR) {
            return Some(m);
        }
        self.managed = None;
        None
    }

    /// Çıkış: canlı değer kabuğun yazdığıysa 1'e dönülür (`Some(1.0)`), değilse dokunulmaz.
    pub fn exit_target(&self, live: f64) -> Option<f64> {
        self.managed.filter(|m| same(*m, live)).map(|_| MIN_FACTOR)
    }

    pub fn view(&self, factor: f64) -> MagnifierView {
        MagnifierView { available: true, factor, max: self.max }
    }

    pub fn status(&self, live: Result<f64, String>) -> Status {
        let (available, factor, reason) = match live {
            Ok(f) => (true, Some(f), None),
            Err(e) => (false, None, Some(e)),
        };
        Status {
            v: 1,
            available,
            factor,
            active: factor.is_some_and(|f| f > MIN_FACTOR + EPS),
            managed: factor.is_some_and(|f| self.managed.is_some_and(|m| same(m, f))),
            step: round3(self.step),
            max: round3(self.max),
            reason,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Hyprland komutları

/// Çalışan Hyprland'in ayar dili: hyprlang `keyword` kabul eder, Lua yalnızca `eval`
/// (ADR-0003 Bulgu 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Hyprlang,
    Lua,
}

/// `j/status` cevabından ayar dili; alan yoksa (eski Hyprland) hyprlang.
pub fn parse_provider(text: &str) -> Option<Provider> {
    #[derive(Deserialize)]
    struct S {
        #[serde(rename = "configProvider")]
        provider: Option<String>,
    }
    let s: S = serde_json::from_str(text).ok()?;
    Some(match s.provider.as_deref() {
        Some("lua") => Provider::Lua,
        _ => Provider::Hyprlang,
    })
}

/// Yazma komutu. Değer yalnızca sayıdır (3 ondalık, sınırlı): enjeksiyon yolu yok.
pub fn write_command(provider: Provider, factor: f64) -> String {
    let f = if factor.is_finite() { round3(factor.clamp(MIN_FACTOR, HYPR_MAX_FACTOR)) } else { MIN_FACTOR };
    match provider {
        Provider::Hyprlang => format!("keyword {OPTION} {f:.3}"),
        Provider::Lua => format!("eval hl.config({{cursor={{zoom_factor={f:.3}}}}})"),
    }
}

/// `j/getoption cursor:zoom_factor` cevabı: `{"option":…, "float": 1.000000, "set": false}`.
/// Hyprland'in alt sınırının altındaki değer 1 sayılır (Hyprland de öyle çizer).
pub fn parse_live(text: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    if v.get("option").and_then(|o| o.as_str()) != Some(OPTION) {
        return None;
    }
    let f = v.get("float").or_else(|| v.get("int")).and_then(serde_json::Value::as_f64)?;
    f.is_finite().then(|| round3(f.clamp(MIN_FACTOR, HYPR_MAX_FACTOR)))
}

/// Canlı çarpan.
pub fn read_live(ctl: &dyn HyprControl) -> Result<f64, String> {
    let text = ctl.request(READ_COMMAND)?;
    parse_live(&text).ok_or_else(|| "Hyprland cursor:zoom_factor değerini vermedi".into()) // i18n: tanı msg yanıtı (çevrilmez)
}

/// Çarpanı yazar (ayar dili her yazımda sorulur: `hypr apply-switch` ile değişebilir).
pub fn write(ctl: &dyn HyprControl, factor: f64) -> Result<(), String> {
    let provider = parse_provider(&ctl.request(PROVIDER_COMMAND)?).unwrap_or(Provider::Hyprlang);
    let reply = ctl.request(&write_command(provider, factor))?;
    if reply.trim() == "ok" {
        Ok(())
    } else {
        Err(reply.chars().filter(|c| !c.is_control()).take(200).collect())
    }
}

/// Tek komut: canlı değeri oku, hedefi hesapla, gerekiyorsa yaz. Sonuç yeni görünüm.
pub fn run(m: &mut Magnifier, ctl: &dyn HyprControl, cmd: Command) -> Result<MagnifierView, String> {
    let live = read_live(ctl)?;
    m.observe(live);
    let target = m.target(cmd, live).map_err(|e| e.to_string())?;
    if same(target, live) {
        m.unchanged(target);
    } else {
        write(ctl, target)?;
        m.wrote(target);
    }
    Ok(m.view(target))
}

/// Kabuk çıkıyor: kendi yazdığı büyütme hâlâ canlıysa 1'e döner. `Ok(true)` = geri alındı.
pub fn release(m: &mut Magnifier, ctl: &dyn HyprControl) -> Result<bool, String> {
    if m.managed().is_none() {
        return Ok(false);
    }
    let live = read_live(ctl)?;
    let target = m.exit_target(live);
    m.observe(live);
    let Some(target) = target else { return Ok(false) };
    write(ctl, target)?;
    m.wrote(target);
    Ok(true)
}

/// Ayar yeniden yüklendi: kabuğun büyütmesi sıfırlandıysa yeniden uygulanır ([`Magnifier::reload_target`]).
/// `Ok(Some(f))` = yeniden yazıldı.
pub fn reapply(m: &mut Magnifier, ctl: &dyn HyprControl) -> Result<Option<f64>, String> {
    if m.managed().is_none() {
        return Ok(None);
    }
    let live = read_live(ctl)?;
    let Some(target) = m.reload_target(live) else { return Ok(None) };
    write(ctl, target)?;
    m.wrote(target);
    Ok(Some(target))
}

/// Çökme sonrası açılış: önceki örneğin işareti (`marker`) canlı değere eşitse 1'e döner.
pub fn recover(m: &mut Magnifier, ctl: &dyn HyprControl, marker: f64) -> Result<bool, String> {
    m.managed = Some(round3(marker));
    release(m, ctl)
}

// ---------------------------------------------------------------------------------------
// Çökme işareti

/// Kabuğun yazdığı büyütmenin kaydı: `$XDG_RUNTIME_DIR/nuxut/magnifier-<imza>`. Hyprland
/// örneğine özgüdür (büyütme Hyprland yeniden başlayınca zaten 1'dir); tmpfs, oturum kapanınca
/// silinir. İçerik tek satır çarpan (`2.000`). Kabuk büyütme yazınca yazılır, 1'e dönünce silinir;
/// çökme sonrası açılışta [`recover`] okur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    path: PathBuf,
}

/// İşaret dosyası okuma sınırı.
const MAX_MARKER_BYTES: u64 = 64;

impl Marker {
    /// Yalnızca güvenli imzayla (Hyprland örnek adı biçimi) ve mutlak çalışma diziniyle.
    pub fn new(runtime_dir: &Path, signature: &str) -> Option<Self> {
        let ok = !signature.is_empty()
            && signature.len() <= 128
            && signature.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        (ok && runtime_dir.is_absolute()).then(|| Self { path: runtime_dir.join("nuxut").join(format!("magnifier-{signature}")) })
    }

    pub fn from_env() -> Option<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty())?;
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        Self::new(Path::new(&dir), &sig)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Kayıtlı çarpan; yoksa ya da bozuksa `None` (bozuk dosya silinir).
    pub fn read(&self) -> Option<f64> {
        let mut text = String::new();
        let file = std::fs::File::open(&self.path).ok()?;
        if file.take(MAX_MARKER_BYTES).read_to_string(&mut text).is_err() {
            self.clear();
            return None;
        }
        let v = text.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > MIN_FACTOR && *v <= HYPR_MAX_FACTOR);
        if v.is_none() {
            self.clear();
        }
        v
    }

    /// Atomik yazım (geçici dosya + `rename`), `0600`; dizin yoksa `0700`.
    pub fn write(&self, factor: f64) -> std::io::Result<()> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let dir = self.path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let tmp = self.path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        writeln!(f, "{:.3}", round3(factor))?;
        f.sync_all()?;
        std::fs::rename(&tmp, &self.path)
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }

    /// Durum makinesinin sahipliğini dosyaya yansıtır.
    pub fn sync(&self, m: &Magnifier) {
        match m.managed() {
            Some(f) => {
                if let Err(e) = self.write(f) {
                    tracing::warn!("büyüteç işareti yazılamadı: {e}");
                }
            }
            None => self.clear(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// Sahte Hyprland: getoption/status/keyword/eval cevaplar, gönderilenleri kaydeder.
    struct Fake {
        zoom: RefCell<f64>,
        provider: &'static str,
        sent: RefCell<Vec<String>>,
        down: bool,
    }

    impl Fake {
        fn new(zoom: f64) -> Self {
            Self { zoom: RefCell::new(zoom), provider: "hyprlang", sent: RefCell::default(), down: false }
        }
        fn writes(&self) -> Vec<String> {
            self.sent.borrow().iter().filter(|c| c.starts_with("keyword") || c.starts_with("eval")).cloned().collect()
        }
    }

    impl HyprControl for Fake {
        fn request(&self, command: &str) -> Result<String, String> {
            if self.down {
                return Err("bağlanılamadı".into());
            }
            self.sent.borrow_mut().push(command.to_owned());
            Ok(match command {
                READ_COMMAND => format!("{{\"option\": \"{OPTION}\", \"float\": {:.6}, \"set\": false }}", self.zoom.borrow()),
                PROVIDER_COMMAND => format!("{{\"configProvider\": \"{}\"}}", self.provider),
                c if c.starts_with("keyword cursor:zoom_factor ") => {
                    *self.zoom.borrow_mut() = c.rsplit(' ').next().unwrap().parse().unwrap();
                    "ok".into()
                }
                c if c.starts_with("eval hl.config({cursor={zoom_factor=") => {
                    let n = c.trim_start_matches("eval hl.config({cursor={zoom_factor=").trim_end_matches("}})");
                    *self.zoom.borrow_mut() = n.parse().unwrap();
                    "ok".into()
                }
                _ => "unknown request".into(),
            })
        }
    }

    fn m() -> Magnifier {
        Magnifier::new(&MagnifierConfig::default())
    }

    #[test]
    fn factor_parsing_rejects_bad_input() {
        assert_eq!(parse_factor("2"), Some(2.0));
        assert_eq!(parse_factor("1.5"), Some(1.5));
        assert_eq!(parse_factor("10"), Some(10.0));
        assert_eq!(parse_factor("1"), Some(1.0));
        for bad in ["", "0.5", "10.01", "-2", "+2", "2,5", "nan", "inf", "1e1", "2x", "1.0.0", "123456789"] {
            assert_eq!(parse_factor(bad), None, "{bad}");
        }
    }

    #[test]
    fn steps_are_multiplicative_and_bounded() {
        let m = m();
        assert_eq!(m.target(Command::In, 1.0), Ok(1.25));
        assert_eq!(m.target(Command::In, 2.0), Ok(2.5));
        assert_eq!(m.target(Command::In, 7.0), Ok(8.0), "tavana kırpılır");
        assert_eq!(m.target(Command::In, 8.0), Ok(8.0));
        // Tavanın üstündeki (kullanıcının) değer `in` ile düşürülmez, tavana iner.
        assert_eq!(m.target(Command::In, 9.5), Ok(8.0));
        assert_eq!(m.target(Command::Out, 1.25), Ok(1.0));
        assert_eq!(m.target(Command::Out, 2.5), Ok(2.0));
        assert_eq!(m.target(Command::Out, 1.01), Ok(1.0), "1'e yapışır");
        assert_eq!(m.target(Command::Out, 1.0), Ok(1.0));
        assert_eq!(m.target(Command::Reset, 5.0), Ok(1.0));
        assert_eq!(m.target(Command::Set(Factor::new(3.333_33)), 1.0), Ok(3.333));
        assert_eq!(m.target(Command::Set(Factor::new(8.0)), 1.0), Ok(8.0));
        assert_eq!(m.target(Command::Set(Factor::new(9.0)), 1.0), Err(PlanError::AboveMax { value: 9.0, max: 8.0 }));
        // Hyprland'in tuhaf alt değeri (0) 1 sayılır.
        assert_eq!(m.target(Command::In, 0.0), Ok(1.25));
    }

    #[test]
    fn toggle_remembers_last_factor() {
        let mut m = m();
        assert_eq!(m.target(Command::Toggle, 1.0), Ok(2.0), "ilk açılış [magnifier] factor");
        m.wrote(3.0);
        assert_eq!(m.target(Command::Toggle, 3.0), Ok(1.0));
        m.wrote(1.0);
        assert_eq!(m.target(Command::Toggle, 1.0), Ok(3.0), "son kullanılan");
        assert_eq!(m.target(Command::On, 1.0), Ok(3.0));
        assert_eq!(m.target(Command::Off, 3.0), Ok(1.0));
    }

    #[test]
    fn config_is_clamped_with_warnings() {
        crate::i18n::use_turkish_for_tests();
        let mut c = MagnifierConfig { step: 5.0, max: 20.0, factor: f32::NAN };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert_eq!((c.step, c.max, c.factor), (2.0, 10.0, 2.0));
        assert_eq!(w.len(), 3, "{w:?}");
        let mut c = MagnifierConfig { step: 1.25, max: 3.0, factor: 4.0 };
        let mut w = Vec::new();
        c.validate(&mut w);
        assert!(w.is_empty(), "anahtarlar tek tek geçerli: {w:?}");
        let m = Magnifier::new(&c);
        assert_eq!(m.target(Command::On, 1.0), Ok(3.0), "factor kullanımda tavana kırpılır");
        let mut again = Vec::new();
        c.validate(&mut again);
        assert!(again.is_empty(), "idempotent");
        let mut d = MagnifierConfig::default();
        d.validate(&mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn commands_are_exact_and_numeric() {
        assert_eq!(write_command(Provider::Hyprlang, 2.0), "keyword cursor:zoom_factor 2.000");
        assert_eq!(write_command(Provider::Lua, 1.5625), "eval hl.config({cursor={zoom_factor=1.563}})");
        assert_eq!(write_command(Provider::Hyprlang, 99.0), "keyword cursor:zoom_factor 10.000");
        assert_eq!(write_command(Provider::Hyprlang, f64::NAN), "keyword cursor:zoom_factor 1.000", "sayı olmayan değer yazılmaz");
        assert_eq!(parse_provider("{\"configProvider\": \"lua\"}"), Some(Provider::Lua));
        assert_eq!(parse_provider("{\"configProvider\": \"hyprlang\"}"), Some(Provider::Hyprlang));
        assert_eq!(parse_provider("{}"), Some(Provider::Hyprlang));
        assert_eq!(parse_provider("unknown request"), None);
        assert_eq!(parse_live("{\"option\": \"cursor:zoom_factor\", \"float\": 2.500000, \"set\": true }"), Some(2.5));
        assert_eq!(parse_live("{\"option\": \"cursor:zoom_factor\", \"float\": 0.000000, \"set\": false }"), Some(1.0));
        assert_eq!(parse_live("{\"option\": \"input:sensitivity\", \"float\": 2.0}"), None);
        assert_eq!(parse_live("no such option"), None);
    }

    /// Hyprlang: `keyword`, Lua: `eval`; okumadan sonra yazma, sınırlar.
    #[test]
    fn run_sends_provider_specific_writes() {
        let fake = Fake::new(1.0);
        let mut m = m();
        let v = run(&mut m, &fake, Command::In).unwrap();
        assert_eq!(v.factor, 1.25);
        assert_eq!(*fake.sent.borrow(), [READ_COMMAND, PROVIDER_COMMAND, "keyword cursor:zoom_factor 1.250"]);
        run(&mut m, &fake, Command::Set(Factor::new(8.0))).unwrap();
        let n = fake.writes().len();
        assert_eq!(run(&mut m, &fake, Command::In).unwrap().factor, 8.0);
        assert_eq!(fake.writes().len(), n, "tavanda yazım yok");
        assert!(run(&mut m, &fake, Command::Set(Factor::new(9.0))).is_err());
        assert_eq!(*fake.zoom.borrow(), 8.0);

        let lua = Fake { provider: "lua", ..Fake::new(1.0) };
        let mut m = self::m();
        run(&mut m, &lua, Command::Set(Factor::new(2.0))).unwrap();
        assert_eq!(lua.writes(), ["eval hl.config({cursor={zoom_factor=2.000}})"]);
        assert_eq!(*lua.zoom.borrow(), 2.0);
    }

    /// Çıkışta yalnızca kabuğun yazdığı değer 1'e döner; kullanıcının elle değiştirdiği kalır.
    #[test]
    fn release_only_reverts_own_zoom() {
        // Kabuk 2.5 yazdı, canlı hâlâ 2.5 → 1.
        let fake = Fake::new(1.0);
        let mut m = m();
        run(&mut m, &fake, Command::Set(Factor::new(2.5))).unwrap();
        assert_eq!(m.managed(), Some(2.5));
        assert_eq!(release(&mut m, &fake), Ok(true));
        assert_eq!(*fake.zoom.borrow(), 1.0);
        assert_eq!(fake.writes().last().unwrap(), "keyword cursor:zoom_factor 1.000");
        assert_eq!(m.managed(), None);
        assert_eq!(release(&mut m, &fake), Ok(false), "ikinci kez bir şey yapılmaz");

        // Kullanıcı sonradan elle 4 yaptı → dokunulmaz.
        let fake = Fake::new(1.0);
        let mut m = self::m();
        run(&mut m, &fake, Command::Set(Factor::new(2.5))).unwrap();
        *fake.zoom.borrow_mut() = 4.0;
        let writes = fake.writes().len();
        assert_eq!(release(&mut m, &fake), Ok(false));
        assert_eq!(fake.writes().len(), writes);
        assert_eq!(*fake.zoom.borrow(), 4.0);

        // Kullanıcının kendi büyütmesi (kabuk hiç yazmadı) → hiç istek yok.
        let fake = Fake::new(3.0);
        let mut m = self::m();
        assert_eq!(release(&mut m, &fake), Ok(false));
        assert!(fake.sent.borrow().is_empty());

        // Zaten canlı olan değere `set` sahiplik yaratmaz.
        let fake = Fake::new(3.0);
        let mut m = self::m();
        run(&mut m, &fake, Command::Set(Factor::new(3.0))).unwrap();
        assert!(fake.writes().is_empty() && m.managed().is_none());
    }

    /// `configreloaded`: kabuğun büyütmesi 1'e döndüyse geri yazılır; kullanıcının ayarı (1'den
    /// farklı başka değer) ezilmez ve sahiplik biter; kabuk büyütmemişse istek yok.
    #[test]
    fn reload_reapplies_only_own_zoom() {
        let fake = Fake::new(1.0);
        let mut m = m();
        run(&mut m, &fake, Command::Set(Factor::new(2.5))).unwrap();
        *fake.zoom.borrow_mut() = 1.0; // reload
        assert_eq!(reapply(&mut m, &fake), Ok(Some(2.5)));
        assert_eq!(*fake.zoom.borrow(), 2.5);
        assert_eq!(m.managed(), Some(2.5));
        let n = fake.writes().len();
        assert_eq!(reapply(&mut m, &fake), Ok(None), "zaten canlı");
        // Kullanıcının ayarı 3 veriyor: ezilmez, sahiplik biter.
        *fake.zoom.borrow_mut() = 3.0;
        assert_eq!(reapply(&mut m, &fake), Ok(None));
        assert_eq!(m.managed(), None);
        assert_eq!(fake.writes().len(), n);
        assert_eq!(*fake.zoom.borrow(), 3.0);
        // Kabuk büyütmemiş: hiç istek yok.
        let fake = Fake::new(1.0);
        let mut m = self::m();
        assert_eq!(reapply(&mut m, &fake), Ok(None));
        assert!(fake.sent.borrow().is_empty());
    }

    #[test]
    fn recover_after_crash() {
        let fake = Fake::new(2.0);
        let mut m = m();
        assert_eq!(recover(&mut m, &fake, 2.0), Ok(true));
        assert_eq!(*fake.zoom.borrow(), 1.0);
        let fake = Fake::new(3.0);
        let mut m = self::m();
        assert_eq!(recover(&mut m, &fake, 2.0), Ok(false), "kullanıcı değiştirmiş");
        assert_eq!(*fake.zoom.borrow(), 3.0);
        assert_eq!(m.managed(), None);
    }

    #[test]
    fn unavailable_hyprland_is_an_error() {
        let fake = Fake { down: true, ..Fake::new(1.0) };
        let mut m = m();
        assert!(run(&mut m, &fake, Command::In).is_err());
        let s = m.status(Err("yok".into()));
        assert!(!s.available && s.factor.is_none());
        let json = m.status(Ok(2.0)).to_json();
        assert!(json.starts_with("{\"v\":1,\"available\":true,\"factor\":2.0,\"active\":true,\"managed\":false"), "{json}");
    }

    #[test]
    fn marker_roundtrip() {
        let dir = std::env::temp_dir().join(format!("nuxut-magnifier-test-{}", std::process::id()));
        let marker = Marker::new(&dir, "abc_123").unwrap();
        assert!(Marker::new(&dir, "../x").is_none());
        assert!(Marker::new(Path::new("rel"), "abc").is_none());
        assert_eq!(marker.read(), None);
        let mut m = m();
        m.wrote(2.5);
        marker.sync(&m);
        assert_eq!(marker.read(), Some(2.5));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(marker.path()).unwrap().permissions().mode() & 0o777, 0o600);
        m.wrote(1.0);
        marker.sync(&m);
        assert_eq!(marker.read(), None);
        std::fs::write(marker.path(), "çöp").unwrap();
        assert_eq!(marker.read(), None);
        assert!(!marker.path().exists(), "bozuk işaret silinir");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

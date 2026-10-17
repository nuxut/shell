//! Sistemin önyükleme düzenini algılama: hangi önyükleyici, Plymouth initramfs'te mi, hangi
//! dosyalar değişecek. Saf: dosya erişimi [`Probe`] üzerinden (kullanıcı tarafı düz okuma,
//! kök yardımcı kendi doğrulanmış dosya katmanı, testler bellek içi harita).
//!
//! Yollar köke göredir ve `/` ile başlamaz (`etc/default/grub`): yardımcı testte geçici bir
//! kökle çalışır.

use crate::{grub, limine, plymouth};

/// Okuma sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    Missing,
    /// İzin yok (kullanıcı tarafında ESP çoğu zaman `0700`): yönetici adımında yeniden bakılır.
    Denied,
    Data(Vec<u8>),
}

/// Dosya sistemi erişimi.
pub trait Probe {
    /// Dosyanın tamamı (en çok `max` bayt). Daha büyük dosya ya da okuma hatası: `None`
    /// (kısaltılmış veri asla dönmez).
    fn read(&self, path: &str, max: usize) -> Option<Read>;
    fn exists(&self, path: &str) -> bool;
    /// Dizindeki adlar (sıralı, en çok `max`); yoksa/okunamazsa boş.
    fn list(&self, dir: &str, max: usize) -> Vec<String>;
}

fn read_text(p: &dyn Probe, path: &str, max: usize) -> Option<Read> {
    p.read(path, max)
}

fn text_of(r: &Option<Read>) -> Option<&str> {
    match r {
        Some(Read::Data(b)) => std::str::from_utf8(b).ok(),
        _ => None,
    }
}

/// Küçük yapılandırma dosyalarının okuma sınırı.
pub const MAX_SMALL: usize = 256 * 1024;
/// grub.cfg sınırı (çok çekirdekli kurulumlarda bile birkaç yüz KB).
pub const MAX_GRUB_CFG: usize = 4 * 1024 * 1024;
/// Limine EFI ikilisi sınırı.
pub const MAX_EFI: usize = 8 * 1024 * 1024;

pub const PLYMOUTH_CONF: &str = "etc/plymouth/plymouthd.conf";
pub const PLYMOUTH_TWO_STEP: &str = "usr/lib/plymouth/two-step.so";
pub const PLYMOUTH_THEMES: &str = "usr/share/plymouth/themes";
pub const PLYMOUTH_SPINNER_KEYMAP: &str = "usr/share/plymouth/themes/spinner/keymap-render.png";
pub const MKINITCPIO: &str = "usr/bin/mkinitcpio";
pub const MKINITCPIO_CONF: &str = "etc/mkinitcpio.conf";
pub const MKINITCPIO_CONF_D: &str = "etc/mkinitcpio.conf.d";
pub const MKINITCPIO_PRESETS: &str = "etc/mkinitcpio.d";
pub const DRACUT: &str = "usr/bin/dracut";
pub const GRUB_MKCONFIG: &str = "usr/bin/grub-mkconfig";
pub const GRUB_DEFAULT: &str = "etc/default/grub";
pub const GRUB_DROPIN_DIR: &str = "etc/default/grub.d";
/// grub.cfg ve tema kökü adayları (Arch `/boot/grub`; bazı dağıtımlarda `/boot/grub2`).
pub const GRUB_DIRS: &[&str] = &["boot/grub", "boot/grub2"];
/// ESP / önyükleme birimi bağlama noktaları.
pub const ESP_ROOTS: &[&str] = &["boot", "efi", "boot/efi"];

/// initramfs üreticisi.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Initramfs {
    /// mkinitcpio; `hook`: HOOKS'ta plymouth var mı; `images`: ön ayarların ürettiği dosyalar
    /// (`None`: ön ayarlar düz değil, çözülemedi).
    Mkinitcpio { hook: bool, images: Option<Vec<String>> },
    Dracut,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlymouthInfo {
    pub installed: bool,
    /// `plymouthd.conf`'taki tema (yoksa dağıtım varsayılanı).
    pub current_theme: Option<String>,
    pub initramfs: Initramfs,
    /// Çekirdek komut satırında `splash` var mı (yoksa Plymouth açılışta görünmez).
    pub splash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GrubInfo {
    /// `boot/grub` ya da `boot/grub2`.
    pub dir: String,
    /// grub.cfg okunabildi mi ve grub-mkconfig üretimi mi (`None`: okunamadı).
    pub generated: Option<bool>,
    pub dropins: bool,
    pub graphical: bool,
    /// Bizimkinden sonra okunan ve `GRUB_THEME` atayan damla (temayı ezer).
    pub overridden_by: Option<String>,
    /// Damlamız ve temamız var mı.
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum LimineConf {
    /// Tek yapılandırma, düzenlenebilir.
    Ok { path: String, applied: bool },
    /// Birden çok aday dosya: hangisinin okunduğunu bilemeyiz.
    Ambiguous { paths: Vec<String> },
    /// Okunamadı (ESP izinleri): yönetici adımında yeniden bakılır.
    Unreadable { path: String },
    /// Sözdizimi tanınmadı / boyut / kodlama.
    Unsupported { path: String, reason: String },
    /// EFI ikilisinde kayıtlı yapılandırma sağlama toplamı.
    Enrolled { path: String, binary: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SystemInfo {
    pub plymouth: PlymouthInfo,
    pub grub: Option<GrubInfo>,
    pub limine: Option<LimineConf>,
    /// systemd-boot (`loader/loader.conf`): temalanamaz, yalnız bilgi.
    pub systemd_boot: bool,
}

/// `HOOKS=(…)` dizisinin son değeri (ana dosya, sonra `conf.d` damlaları). Çok satırlı parantez
/// desteklenir.
pub fn mkinitcpio_hooks(texts: &[&str]) -> Option<Vec<String>> {
    let mut found = None;
    for text in texts {
        let mut iter = text.lines();
        while let Some(line) = iter.next() {
            let t = line.trim();
            let Some(rest) = t.strip_prefix("HOOKS=(") else { continue };
            let mut body = rest.to_owned();
            while !body.contains(')') {
                match iter.next() {
                    Some(next) => {
                        body.push(' ');
                        body.push_str(next.trim());
                    }
                    None => break,
                }
            }
            let inner = body.split(')').next().unwrap_or("");
            found = Some(inner.split_whitespace().map(|s| s.trim_matches(|c| c == '"' || c == '\'').to_owned()).collect());
        }
    }
    found
}

pub fn has_plymouth_hook(hooks: &[String]) -> bool {
    hooks.iter().any(|h| h == "plymouth" || h == "sd-plymouth")
}

/// Çekirdek komut satırında `splash` sözcüğü.
pub fn cmdline_has_splash(cmdline: &str) -> bool {
    cmdline.split_whitespace().any(|w| w == "splash")
}

/// mkinitcpio ön ayarının ürettiği dosyalar (`*_image=` / `*_uki=`). Değer `$`/`` ` `` içeriyor
/// ya da mutlak değilse çözülemez → `None`.
pub fn preset_outputs(text: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            continue;
        }
        let Some((key, value)) = t.split_once('=') else { continue };
        if !(key.ends_with("_image") || key.ends_with("_uki")) || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            continue;
        }
        let v = value.trim().trim_matches(|c| c == '"' || c == '\'');
        if v.is_empty() {
            continue;
        }
        if v.contains('$') || v.contains('`') || !v.starts_with('/') || v.contains("/../") {
            return None;
        }
        out.push(v.to_owned());
    }
    Some(out)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") }
}

fn detect_plymouth(p: &dyn Probe, cmdline: &str) -> PlymouthInfo {
    let installed = p.exists(PLYMOUTH_TWO_STEP) && p.exists(PLYMOUTH_THEMES);
    let conf = read_text(p, PLYMOUTH_CONF, MAX_SMALL);
    let current_theme = text_of(&conf).and_then(plymouth::daemon_theme);
    let initramfs = if p.exists(MKINITCPIO) {
        let main = read_text(p, MKINITCPIO_CONF, MAX_SMALL);
        let mut texts: Vec<Option<Read>> = vec![main];
        for name in p.list(MKINITCPIO_CONF_D, 64).into_iter().filter(|n| n.ends_with(".conf")) {
            texts.push(read_text(p, &join(MKINITCPIO_CONF_D, &name), MAX_SMALL));
        }
        let strs: Vec<&str> = texts.iter().filter_map(text_of).collect();
        let hook = mkinitcpio_hooks(&strs).is_some_and(|h| has_plymouth_hook(&h));
        let mut images = Some(Vec::new());
        for name in p.list(MKINITCPIO_PRESETS, 64).into_iter().filter(|n| n.ends_with(".preset")) {
            let r = read_text(p, &join(MKINITCPIO_PRESETS, &name), MAX_SMALL);
            match (text_of(&r).and_then(preset_outputs), images.as_mut()) {
                (Some(v), Some(all)) => all.extend(v),
                _ => images = None,
            }
        }
        Initramfs::Mkinitcpio { hook, images }
    } else if p.exists(DRACUT) {
        Initramfs::Dracut
    } else {
        Initramfs::Unknown
    };
    PlymouthInfo { installed, current_theme, initramfs, splash: cmdline_has_splash(cmdline) }
}

fn detect_grub(p: &dyn Probe) -> Option<GrubInfo> {
    if !p.exists(GRUB_MKCONFIG) {
        return None;
    }
    let dir = GRUB_DIRS.iter().find(|d| p.exists(&join(d, "grub.cfg"))).or_else(|| GRUB_DIRS.iter().find(|d| p.exists(d)))?;
    let cfg = read_text(p, &join(dir, "grub.cfg"), MAX_GRUB_CFG);
    let generated = match &cfg {
        Some(Read::Data(_)) => Some(text_of(&cfg).is_some_and(grub::is_generated)),
        _ => None,
    };
    let script = read_text(p, GRUB_MKCONFIG, MAX_SMALL);
    let dropins = text_of(&script).is_some_and(grub::mkconfig_reads_dropins);
    let main = read_text(p, GRUB_DEFAULT, MAX_SMALL);
    let names: Vec<String> = p.list(GRUB_DROPIN_DIR, 64).into_iter().filter(|n| n.ends_with(".cfg")).collect();
    let drops: Vec<(String, Option<Read>)> = names.iter().map(|n| (n.clone(), read_text(p, &join(GRUB_DROPIN_DIR, n), MAX_SMALL))).collect();
    let mut texts: Vec<&str> = text_of(&main).into_iter().collect();
    texts.extend(drops.iter().filter_map(|(_, r)| text_of(r)));
    let graphical = grub::graphical_terminal(&texts);
    let overridden_by = drops
        .iter()
        .filter(|(n, _)| n.as_str() > grub::DROPIN_NAME)
        .find(|(_, r)| text_of(r).is_some_and(|t| grub::shell_var(&[t], "GRUB_THEME").is_some()))
        .map(|(n, _)| join(GRUB_DROPIN_DIR, n));
    let applied = names.iter().any(|n| n == grub::DROPIN_NAME) && p.exists(&format!("{dir}/{}/theme.txt", grub::THEME_SUBDIR));
    Some(GrubInfo { dir: (*dir).to_owned(), generated, dropins, graphical, overridden_by, applied })
}

/// Limine yapılandırma adayları (var olanlar, tekilleştirilmiş).
pub fn limine_candidates(p: &dyn Probe) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut roots: Vec<&str> = vec![""];
    roots.extend(ESP_ROOTS);
    for root in roots {
        for c in limine::VOLUME_CANDIDATES {
            paths.push(join(root, c));
        }
        let efi = join(root, "EFI");
        for sub in p.list(&efi, 64) {
            paths.push(join(&join(&efi, &sub), "limine.conf"));
        }
    }
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        if !out.contains(&path) && p.exists(&path) {
            out.push(path);
        }
    }
    out
}

/// ESP'lerdeki Limine EFI ikilileri (sağlama toplamı denetimi için).
pub fn limine_binaries(p: &dyn Probe) -> Vec<String> {
    let mut out = Vec::new();
    let mut roots: Vec<&str> = vec![""];
    roots.extend(ESP_ROOTS);
    for root in roots {
        let efi = join(root, "EFI");
        for sub in p.list(&efi, 64) {
            let dir = join(&efi, &sub);
            for name in p.list(&dir, 64) {
                if name.to_ascii_lowercase().ends_with(".efi") {
                    let path = join(&dir, &name);
                    if !out.contains(&path) {
                        out.push(path);
                    }
                }
            }
        }
    }
    out
}

fn detect_limine(p: &dyn Probe) -> Option<LimineConf> {
    let found = limine_candidates(p);
    let path = match found.as_slice() {
        [] => return None,
        [one] => one.clone(),
        many => return Some(LimineConf::Ambiguous { paths: many.to_vec() }),
    };
    for bin in limine_binaries(p) {
        if let Some(Read::Data(bytes)) = p.read(&bin, MAX_EFI)
            && limine::config_checksum_enrolled(&bytes) == Some(true)
        {
            return Some(LimineConf::Enrolled { path, binary: bin });
        }
    }
    match p.read(&path, limine::MAX_CONF) {
        Some(Read::Data(bytes)) => match limine::decode(&bytes).and_then(|t| {
            let applied = limine::has_block(t);
            limine::validate(&limine::remove_block(t)?).map(|()| applied)
        }) {
            Ok(applied) => Some(LimineConf::Ok { path, applied }),
            Err(e) => Some(LimineConf::Unsupported { path, reason: e.to_string() }),
        },
        Some(Read::Denied) => Some(LimineConf::Unreadable { path }),
        Some(Read::Missing) => None,
        None => Some(LimineConf::Unsupported { path, reason: limine::EditError::TooLarge.to_string() }),
    }
}

fn detect_systemd_boot(p: &dyn Probe) -> bool {
    ESP_ROOTS.iter().any(|r| p.exists(&join(r, "loader/loader.conf")))
}

/// Tüm algılama. `cmdline`: `/proc/cmdline` içeriği.
pub fn system(p: &dyn Probe, cmdline: &str) -> SystemInfo {
    SystemInfo {
        plymouth: detect_plymouth(p, cmdline),
        grub: detect_grub(p),
        limine: detect_limine(p),
        systemd_boot: detect_systemd_boot(p),
    }
}

/// Bellek içi sahte dosya sistemi (testler ve önizleme örnekleri).
#[derive(Debug, Default, Clone)]
pub struct MapProbe {
    pub files: std::collections::BTreeMap<String, Read>,
}

impl MapProbe {
    pub fn with(mut self, path: &str, data: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.to_owned(), Read::Data(data.into()));
        self
    }

    pub fn denied(mut self, path: &str) -> Self {
        self.files.insert(path.to_owned(), Read::Denied);
        self
    }
}

impl Probe for MapProbe {
    fn read(&self, path: &str, max: usize) -> Option<Read> {
        match self.files.get(path) {
            Some(Read::Data(d)) if d.len() > max => None,
            Some(r) => Some(r.clone()),
            None => Some(Read::Missing),
        }
    }

    fn exists(&self, path: &str) -> bool {
        let dir = format!("{path}/");
        self.files.contains_key(path) || self.files.keys().any(|k| k.starts_with(&dir))
    }

    fn list(&self, dir: &str, max: usize) -> Vec<String> {
        let prefix = format!("{dir}/");
        let mut out: Vec<String> = self
            .files
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_owned())
            .collect();
        out.dedup();
        out.truncate(max);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MKCONFIG: &str = "for dropin in ${sysconfdir}/default/grub.d/*.cfg ; do\n";
    const CFG: &str = "# It is automatically generated by grub-mkconfig using templates\nmenuentry 'Arch' {\n}\n";

    fn arch_grub() -> MapProbe {
        MapProbe::default()
            .with(GRUB_MKCONFIG, MKCONFIG)
            .with("boot/grub/grub.cfg", CFG)
            .with(GRUB_DEFAULT, "GRUB_TIMEOUT=5\n")
            .with(MKINITCPIO, "")
            .with(MKINITCPIO_CONF, "HOOKS=(base udev autodetect modconf kms keyboard\n  plymouth block filesystems fsck)\n")
            .with("etc/mkinitcpio.d/linux.preset", "PRESETS=('default')\ndefault_image=\"/boot/initramfs-linux.img\"\n#fallback_image=\"/x\"\n")
            .with(PLYMOUTH_TWO_STEP, "")
            .with("usr/share/plymouth/themes/spinner/spinner.plymouth", "")
            .with(PLYMOUTH_CONF, "[Daemon]\nTheme=bgrt\n")
    }

    #[test]
    fn arch_with_grub_and_plymouth() {
        let info = system(&arch_grub(), "BOOT_IMAGE=/vmlinuz root=x quiet splash");
        assert!(info.plymouth.installed && info.plymouth.splash);
        assert_eq!(info.plymouth.current_theme.as_deref(), Some("bgrt"));
        assert_eq!(info.plymouth.initramfs, Initramfs::Mkinitcpio { hook: true, images: Some(vec!["/boot/initramfs-linux.img".into()]) });
        let g = info.grub.unwrap();
        assert_eq!(g.dir, "boot/grub");
        assert_eq!(g.generated, Some(true));
        assert!(g.dropins && g.graphical && !g.applied && g.overridden_by.is_none());
        assert_eq!(info.limine, None);
        assert!(!info.systemd_boot);
    }

    #[test]
    fn grub_override_and_text_terminal() {
        let p = arch_grub()
            .with("etc/default/grub.d/99-other.cfg", "GRUB_THEME=/x/theme.txt\n")
            .with("etc/default/grub.d/10-term.cfg", "GRUB_TERMINAL_OUTPUT=console\n");
        let g = system(&p, "").grub.unwrap();
        assert_eq!(g.overridden_by.as_deref(), Some("etc/default/grub.d/99-other.cfg"));
        assert!(!g.graphical);
    }

    #[test]
    fn no_hook_no_splash_unknown_preset() {
        let p = arch_grub()
            .with(MKINITCPIO_CONF, "HOOKS=(base udev)\n")
            .with("etc/mkinitcpio.conf.d/x.conf", "HOOKS=(base systemd sd-plymouth)\n")
            .with("etc/mkinitcpio.d/linux.preset", "default_image=\"$BOOT/initramfs.img\"\n");
        let pl = system(&p, "quiet").plymouth;
        assert!(!pl.splash);
        assert_eq!(pl.initramfs, Initramfs::Mkinitcpio { hook: true, images: None }, "conf.d damlası HOOKS'u ezer");
    }

    #[test]
    fn limine_cases() {
        let conf = "timeout: 3\n/Arch\n    protocol: linux\n";
        let p = MapProbe::default().with("boot/EFI/limine/limine.conf", conf).with("boot/EFI/limine/BOOTX64.EFI", unenrolled());
        assert_eq!(system(&p, "").limine, Some(LimineConf::Ok { path: "boot/EFI/limine/limine.conf".into(), applied: false }));
        let p2 = p.clone().with("boot/limine.conf", conf);
        assert!(matches!(system(&p2, "").limine, Some(LimineConf::Ambiguous { .. })));
        let mut enrolled = unenrolled();
        let n = enrolled.len();
        enrolled[n - 10] = b'f';
        let p3 = p.clone().with("boot/EFI/limine/BOOTX64.EFI", enrolled);
        assert!(matches!(system(&p3, "").limine, Some(LimineConf::Enrolled { .. })));
        let p4 = MapProbe::default().denied("efi/limine.conf");
        assert_eq!(system(&p4, "").limine, Some(LimineConf::Unreadable { path: "efi/limine.conf".into() }));
        let p5 = MapProbe::default().with("boot/limine.conf", "TIMEOUT=5\n:Arch\n");
        assert!(matches!(system(&p5, "").limine, Some(LimineConf::Unsupported { .. })));
    }

    fn unenrolled() -> Vec<u8> {
        let mut v = b"MZ...++CONFIG_B2SUM_SIGNATURE++".to_vec();
        v.extend(std::iter::repeat_n(b'0', 128));
        v
    }

    #[test]
    fn systemd_boot_detected() {
        let p = MapProbe::default().with("efi/loader/loader.conf", "timeout 3\n");
        assert!(system(&p, "").systemd_boot);
    }

    #[test]
    fn preset_parsing() {
        assert_eq!(preset_outputs("default_uki='/efi/EFI/Linux/arch.efi'\n"), Some(vec!["/efi/EFI/Linux/arch.efi".into()]));
        assert_eq!(preset_outputs("default_image=relative.img\n"), None);
        assert_eq!(preset_outputs("default_image=/boot/../etc/x\n"), None);
        assert_eq!(preset_outputs("ALL_kver=\"/boot/vmlinuz-linux\"\n"), Some(vec![]));
    }
}

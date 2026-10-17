//! Nuxut oturumunun portal seçim dosyası (`nuxut-portals.conf`) — kullanıcının Hyprland
//! tercihlerinden türetilir (docs/design/portal.md "Seçim dosyası").
//!
//! Nuxut oturumunda `XDG_CURRENT_DESKTOP=Nuxut:Hyprland`. xdg-desktop-portal 1.22 her yapılandırma
//! dizininde masaüstü listesi sırasıyla `<masaüstü>-portals.conf`, sonra `portals.conf` arar ve o
//! dizinden yalnızca İLK yüklenebilen dosyayı alır (`src/xdp-portal-config.c`,
//! `load_config_directory`). Bu yüzden `$XDG_CONFIG_HOME/xdg-desktop-portal/nuxut-portals.conf`
//! varken aynı dizindeki `hyprland-portals.conf`/`portals.conf` okunmaz. Kullanıcının oradaki
//! seçimleri (ör. `FileChooser = kde`) kaybolmasın diye dosya, Nuxut olmasaydı Hyprland için
//! seçilecek dosyanın kopyasıdır; tek fark `org.freedesktop.impl.portal.Settings` başına `nuxut`.
//!
//! Dizin sırası (`load_portal_configurations`): `$XDG_CONFIG_HOME`, `$XDG_CONFIG_DIRS`,
//! sysconfdir (`/etc`), `$XDG_DATA_HOME`, `$XDG_DATA_DIRS`, datadir (`/usr/share`); her birinin
//! altında `xdg-desktop-portal/`. Hepsi sırayla zincirlenir (alt katmanlar Nuxut oturumunda da
//! okunur); ilk bulunan dosyanın kopyası en üste konunca sonuç Hyprland oturumuyla aynı kalır.
//!
//! Kullanıcının dosyaları yalnızca OKUNUR. Hedef yalnızca yoksa ya da ilk satırı
//! [`PORTALS_CONF_PREFIX`] ile başlıyorsa (Nuxut'un) yazılır; işaret silinmişse kullanıcı sahiplenmiştir.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

/// Sahiplik öneki (install.sh `PORTALS_CONF_PREFIX` ile birebir aynı): ilk satır bununla
/// başladıkça dosya Nuxut'undur. Sabit önek: işaretin açıklama metni sürümler arasında
/// değişebilir, sahiplik değişmez.
pub const PORTALS_CONF_PREFIX: &str = "# nuxut-managed: portals";
/// Yazılan ilk satır: önek + açıklama.
pub const PORTALS_CONF_MARK: &str = "# nuxut-managed: portals — Nuxut oturumunun portal seçimi \
                                     (XDG_CURRENT_DESKTOP=Nuxut:Hyprland; man portals.conf).";
/// Hedef dosya adı (`XDG_CURRENT_DESKTOP`'taki `Nuxut` → küçük harf).
pub const FILE_NAME: &str = "nuxut-portals.conf";
/// Taklit edilen masaüstü (Nuxut yalnızca Hyprland üzerinde çalışır; xdp küçük harfe çevirir).
pub const HOST_DESKTOPS: &[&str] = &["hyprland"];
/// Başına `nuxut` eklenen arayüz.
pub const SETTINGS_KEY: &str = "org.freedesktop.impl.portal.Settings";
/// Dosya seçici arayüzü: Nuxut varsayılandır (kullanıcı ilkesi "Nuxut yapabiliyorsa varsayılan o",
/// 2026-10-16; docs/design/file-chooser.md). Değer `nuxut` + kaynaktaki açık seçim (ör.
/// `kde`; yoksa `default`) — önceki seçim yedek olur, açık seçimin satırı [`BACKUP_GROUP`]'ta saklanır.
pub const FILE_CHOOSER_KEY: &str = "org.freedesktop.impl.portal.FileChooser";
/// Devralınan açık seçimin yedek grubu: xdp yalnız `[preferred]`'ı okur, bu grup seçimi etkilemez.
pub const BACKUP_GROUP: &str = "X-Nuxut-Backup";
/// Dosya seçici devralma kapalı (`--no-filechooser`): dosyada bu satır durdukça yeniden üretim
/// (install.sh dahil) kapalı tutar; `--filechooser` yeniden açar.
pub const FILE_CHOOSER_OFF: &str = "# nuxut-filechooser: off";
/// Bizim arka ucumuzun `.portal` adı.
pub const BACKEND: &str = "nuxut";
/// Yapılandırma dizinlerinin alt dizini (`XDP_SUBDIR`).
const SUBDIR: &str = "xdg-desktop-portal";
/// Seçim grubu.
const GROUP: &str = "preferred";
/// Kaynak yokken: xdp'nin Hyprland için yapılandırmasız davranışı (`UseIn` → hyprland,
/// son çare gtk) ve bugünkü varsayılan. FileChooser sabitlenmez: hyprland onu sunmaz, gtk'ye
/// zaten `default` ile düşer — ayrı satır yalnızca kullanıcının seçimini gölgelerdi.
const FALLBACK_BODY: &str = "[preferred]\ndefault=hyprland;gtk\n";

/// xdg-desktop-portal'ın taradığı kök dizinler (her birinin altında `xdg-desktop-portal/`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDirs {
    pub config_home: PathBuf,
    pub config_dirs: Vec<PathBuf>,
    /// `${sysconfdir}` (Arch paketi: `/etc`).
    pub sysconfdir: PathBuf,
    pub data_home: PathBuf,
    pub data_dirs: Vec<PathBuf>,
    /// `${datadir}` (Arch paketi: `/usr/share`).
    pub datadir: PathBuf,
}

impl SearchDirs {
    /// GLib'in `g_get_user_config_dir` & co. kurallarıyla: boş/tanımsız değişken varsayılana düşer.
    pub fn from_env() -> Option<Self> {
        Self::from_vars(|k| std::env::var_os(k))
    }

    /// Ortam okuyucusundan (testler için).
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Option<Self> {
        let set = |k: &str| var(k).filter(|v| !v.is_empty());
        let home = set("HOME").map(PathBuf::from);
        let user = |k: &str, rel: &str| set(k).map(PathBuf::from).or_else(|| home.as_ref().map(|h| h.join(rel)));
        let list = |k: &str, def: &str| -> Vec<PathBuf> {
            let v = set(k).unwrap_or_else(|| def.into());
            std::env::split_paths(&v).filter(|p| !p.as_os_str().is_empty()).collect()
        };
        Some(Self {
            config_home: user("XDG_CONFIG_HOME", ".config")?,
            config_dirs: list("XDG_CONFIG_DIRS", "/etc/xdg"),
            sysconfdir: PathBuf::from("/etc"),
            data_home: user("XDG_DATA_HOME", ".local/share")?,
            data_dirs: list("XDG_DATA_DIRS", "/usr/local/share:/usr/share"),
            datadir: PathBuf::from("/usr/share"),
        })
    }

    /// xdp'nin arama sırasıyla `…/xdg-desktop-portal` dizinleri.
    pub fn ordered(&self) -> Vec<PathBuf> {
        let mut out = vec![self.config_home.join(SUBDIR)];
        out.extend(self.config_dirs.iter().map(|d| d.join(SUBDIR)));
        out.push(self.sysconfdir.join(SUBDIR));
        out.push(self.data_home.join(SUBDIR));
        out.extend(self.data_dirs.iter().map(|d| d.join(SUBDIR)));
        out.push(self.datadir.join(SUBDIR));
        out
    }

    /// Nuxut'un yazdığı dosya: `$XDG_CONFIG_HOME/xdg-desktop-portal/nuxut-portals.conf`.
    pub fn target(&self) -> PathBuf {
        self.config_home.join(SUBDIR).join(FILE_NAME)
    }
}

/// Türetmenin kaynağı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: PathBuf,
    pub text: String,
}

/// Nuxut olmasaydı `desktops` (küçük harf) için xdp'nin ilk yükleyeceği dosya: dizin sırasıyla,
/// her dizinde `<masaüstü>-portals.conf`'lar sonra `portals.conf`; yalnızca [`load`]'un kabul
/// ettiği (GKeyFile ayrıştırır ve `[preferred]`'da en az bir anahtar var) dosya sayılır.
/// `nuxut-portals.conf` hiç aranmaz (masaüstü listesinde `nuxut` yok).
pub fn find_source(dirs: &SearchDirs, desktops: &[&str]) -> Option<Source> {
    let names: Vec<String> = desktops
        .iter()
        .filter(|d| !d.eq_ignore_ascii_case(BACKEND))
        .map(|d| format!("{}-portals.conf", d.to_ascii_lowercase()))
        .chain(std::iter::once("portals.conf".to_owned()))
        .collect();
    for dir in dirs.ordered() {
        for name in &names {
            let path = dir.join(name);
            let Ok(text) = fs::read_to_string(&path) else { continue };
            if load(&text).is_some() {
                return Some(Source { path, text });
            }
        }
    }
    None
}

/// Ayrıştırılmış satır.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// Boş satır ya da yorum.
    Other,
    Group(String),
    Pair { key: String, value: String },
}

/// GKeyFile'ın satır kurallarına yakın ayrıştırma; GKeyFile'ın reddedeceği dosyada `None`
/// (xdp o dosyayı atlar). Baştaki boşluk atılır; `#` yorum; `[ad]` grup; `anahtar = değer`
/// (anahtarın sonundaki, değerin başındaki boşluk atılır); gruptan önceki çift ya da `=`'siz
/// satır hata.
fn parse(text: &str) -> Option<Vec<Line>> {
    let mut out = Vec::new();
    let mut in_group = false;
    for raw in text.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            out.push(Line::Other);
        } else if let Some(rest) = line.strip_prefix('[') {
            let name = rest.trim_end().strip_suffix(']')?;
            if name.is_empty() || name.contains(['[', ']']) {
                return None;
            }
            in_group = true;
            out.push(Line::Group(name.to_owned()));
        } else {
            let (key, value) = line.split_once('=')?;
            let key = key.trim_end();
            if !in_group || key.is_empty() {
                return None;
            }
            out.push(Line::Pair { key: key.to_owned(), value: value.trim_start().to_owned() });
        }
    }
    Some(out)
}

/// `[preferred]` anahtarları (aynı grup birden çok kez olabilir; GKeyFile birleştirir, aynı
/// anahtarda sonuncusu geçerli). xdp'nin dosyayı kabul ettiği durumda en az bir anahtar döner.
fn load(text: &str) -> Option<Vec<(String, String)>> {
    let lines = parse(text)?;
    let mut keys: Vec<(String, String)> = Vec::new();
    let mut group = "";
    for line in &lines {
        match line {
            Line::Group(g) => group = g,
            Line::Pair { key, value } if group == GROUP => {
                keys.retain(|(k, _)| k != key);
                keys.push((key.clone(), value.clone()));
            }
            _ => {}
        }
    }
    (!keys.is_empty()).then_some(keys)
}

/// `g_key_file_get_string_list` gibi: kaçışsız `;` ile böl, kaçışları çöz, sondaki boş öğeyi at.
/// Boşluk toleransı için öğeler kırpılır, boşlar atılır.
fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('s') => cur.push(' '),
                Some('n') => cur.push('\n'),
                Some('t') => cur.push('\t'),
                Some('r') => cur.push('\r'),
                Some(o) => cur.push(o),
                None => {}
            },
            ';' => items.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    items.push(cur);
    items.into_iter().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
}

/// Settings değeri: `nuxut` + kaynaktaki Settings (yoksa `default`) listesi, tekrarsız.
/// `none` atılır: xdp listede `none` görürse arayüzü tümden kapatır (`nuxut` da çalışmazdı).
pub fn settings_value(keys: &[(String, String)]) -> String {
    let base = keys
        .iter()
        .find(|(k, _)| k == SETTINGS_KEY)
        .or_else(|| keys.iter().find(|(k, _)| k == "default"))
        .map(|(_, v)| split_list(v))
        .unwrap_or_default();
    let mut out = vec![BACKEND.to_owned()];
    for item in base {
        if item != "none" && !out.contains(&item) {
            out.push(item);
        }
    }
    out.join(";")
}

/// FileChooser değeri: `nuxut` + kaynaktaki açık FileChooser listesi (yoksa `default` listesi),
/// tekrarsız, `none` atılır. Açık seçim (ör. `kde`) yedek arka uç olarak kalır.
pub fn file_chooser_value(keys: &[(String, String)]) -> String {
    let base = keys
        .iter()
        .find(|(k, _)| k == FILE_CHOOSER_KEY)
        .or_else(|| keys.iter().find(|(k, _)| k == "default"))
        .map(|(_, v)| split_list(v))
        .unwrap_or_default();
    let mut out = vec![BACKEND.to_owned()];
    for item in base {
        if item != "none" && !out.contains(&item) {
            out.push(item);
        }
    }
    out.join(";")
}

/// Kaynağın metni: yönetilen anahtarlar (Settings; `file_chooser` ise FileChooser) tek satırla
/// değişir (ilk geçtiği yerde; yoksa ilk `[preferred]` bölümünün son anahtarından sonra); geri
/// kalan her satır (yorum, bilinmeyen anahtar, diğer gruplar) olduğu gibi kalır. Devralınan açık
/// FileChooser seçimi değişiyorsa özgün satır sonda [`BACKUP_GROUP`]'ta saklanır.
fn rewrite(text: &str, file_chooser: bool) -> Option<String> {
    let lines = parse(text)?;
    let keys = load(text)?;
    let mut managed = vec![(SETTINGS_KEY, settings_value(&keys))];
    let mut backup = None;
    if file_chooser {
        let value = file_chooser_value(&keys);
        if let Some((_, old)) = keys.iter().find(|(k, _)| k == FILE_CHOOSER_KEY)
            && split_list(old).join(";") != value
        {
            backup = Some(old.clone());
        }
        managed.push((FILE_CHOOSER_KEY, value));
    }
    let raw: Vec<&str> = text.lines().collect();
    let mut group = "";
    let mut first = vec![None; managed.len()];
    let mut first_section_last_pair = None;
    let mut seen_preferred = false;
    let mut first_section_open = false;
    for (i, line) in lines.iter().enumerate() {
        match line {
            Line::Group(g) => {
                group = g;
                first_section_open = g == GROUP && !seen_preferred;
                seen_preferred |= g == GROUP;
            }
            Line::Pair { key, .. } if group == GROUP => {
                if let Some(m) = managed.iter().position(|(k, _)| k == key) {
                    first[m].get_or_insert(i);
                }
                if first_section_open {
                    first_section_last_pair = Some(i);
                }
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(text.len() + 256);
    let mut group = "";
    for (i, line) in lines.iter().enumerate() {
        if let Line::Group(g) = line {
            group = g;
        }
        let managed_at = match line {
            Line::Pair { key, .. } if group == GROUP => managed.iter().position(|(k, _)| k == key),
            _ => None,
        };
        match managed_at {
            Some(m) if first[m] == Some(i) => out.push_str(&format!("{}={}\n", managed[m].0, managed[m].1)),
            Some(_) => {}
            None => {
                out.push_str(raw[i]);
                out.push('\n');
            }
        }
        // Çapa yönetilen bir satır da olabilir (bu makinede: son anahtar FileChooser).
        if Some(i) == first_section_last_pair {
            for (m, (key, value)) in managed.iter().enumerate() {
                if first[m].is_none() {
                    out.push_str(&format!("{key}={value}\n"));
                }
            }
        }
    }
    if let Some(old) = backup {
        out.push_str(&format!(
            "\n# Nuxut dosya seçicisi devraldı; kaynaktaki önceki seçim (xdp bu grubu okumaz):\n[{BACKUP_GROUP}]\n{FILE_CHOOSER_KEY}={old}\n"
        ));
    }
    Some(out)
}

/// Var olan (Nuxut'un) dosyada dosya seçici devralma kapatılmış mı ([`FILE_CHOOSER_OFF`]).
pub fn file_chooser_disabled(existing: &str) -> bool {
    existing.lines().any(|l| l.trim_end() == FILE_CHOOSER_OFF)
}

/// Dosyanın tam içeriği: işaret + kaynak + yeniden üretim yönergesi, ardından kaynağın
/// dönüştürülmüş metni (kaynak yoksa xdp'nin Hyprland varsayılanı).
pub fn render(source: Option<&Source>) -> String {
    render_with(source, DEFAULT_REGEN)
}

/// Varsayılan yeniden üretim komutu (install.sh'ın `$LIBEXEC`'i).
pub const DEFAULT_REGEN: &str = "~/.local/lib/nuxut/xdg-desktop-portal-nuxut --write-portals-conf";

/// [`render`]; `regen` yönerge satırındaki komut (çalışan ikilinin yolu).
pub fn render_with(source: Option<&Source>, regen: &str) -> String {
    render_opts(source, regen, true)
}

/// [`render_with`]; `file_chooser`: Nuxut dosya seçicisi FileChooser seçimini devralsın mı
/// (`--no-filechooser` kapatır; kapalılık dosyada [`FILE_CHOOSER_OFF`] olarak kalır).
pub fn render_opts(source: Option<&Source>, regen: &str, file_chooser: bool) -> String {
    let regen = regen.replace('\n', "?");
    let body = source.and_then(|s| rewrite(&s.text, file_chooser));
    let origin = match (source, &body) {
        (Some(s), Some(_)) => format!("türetildi: {}", s.path.display().to_string().replace('\n', "?")),
        _ => "kaynak yok: xdg-desktop-portal'ın Hyprland varsayılanı (hyprland, son çare gtk)".to_owned(),
    };
    let body = body.unwrap_or_else(|| {
        let keys = load(FALLBACK_BODY).unwrap_or_default();
        let chooser = if file_chooser { format!("{FILE_CHOOSER_KEY}={}\n", file_chooser_value(&keys)) } else { String::new() };
        format!("{FALLBACK_BODY}{SETTINGS_KEY}={}\n{chooser}", settings_value(&keys))
    });
    // Dosya seçici durumu ve tek komutla geri alma / yeniden açma.
    let base = regen.trim_end_matches(" --no-filechooser").trim_end_matches(" --filechooser");
    let chooser = if file_chooser {
        format!(
            "# Dosya seçici: Nuxut ({FILE_CHOOSER_KEY}={BACKEND};<önceki seçim>; açık seçim [{BACKUP_GROUP}]'ta yedekli).\n\
             # Geri almak (önceki seçime dönmek) için:  {base} --no-filechooser\n"
        )
    } else {
        format!(
            "{FILE_CHOOSER_OFF}\n\
             # Dosya seçici devralınmadı (kaynaktaki seçim geçerli). Nuxut seçicisi için:  {base} --filechooser\n"
        )
    };
    format!(
        "{PORTALS_CONF_MARK}\n\
         # Nuxut üretir; Nuxut olmasaydı Hyprland oturumunda seçilecek dosyanın kopyası; farklar:\n\
         # {SETTINGS_KEY} başına '{BACKEND}' (görünüm: koyu/açık, vurgu) ve dosya seçici (aşağıda).\n\
         # {origin}\n\
         {chooser}\
         # O dosyayı değiştirirseniz yeniden üretin (sonraki girişte geçerli):\n\
         #   {base}   (ya da install.sh)\n\
         # Bu ilk satır durdukça dosya yeniden üretilir; kendi seçiminiz için ilk satırı silip düzenleyin.\n\
         {body}"
    )
}

/// [`write`] sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Yazıldı (yeni ya da Nuxut'un önceki dosyası).
    Written,
    /// İçerik zaten aynıydı; dokunulmadı.
    Unchanged,
    /// İlk satır işareti yok: kullanıcının dosyası, dokunulmadı.
    UserOwned,
}

/// Hedef Nuxut'un mu (yok ya da ilk satırı [`PORTALS_CONF_PREFIX`] ile başlıyor)?
pub fn is_ours(target: &Path) -> io::Result<bool> {
    match fs::read_to_string(target) {
        Ok(text) => Ok(text.lines().next().is_some_and(|l| l.starts_with(PORTALS_CONF_PREFIX))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        // UTF-8 olmayan dosya Nuxut'un olamaz.
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Ok(false),
        Err(e) => Err(e),
    }
}

/// Hedef yoksa ya da Nuxut'unsa atomik yazar (aynı dizinde geçici dosya + `rename`).
pub fn write(target: &Path, content: &str) -> io::Result<Outcome> {
    if !is_ours(target)? {
        return Ok(Outcome::UserOwned);
    }
    if fs::read_to_string(target).is_ok_and(|old| old == content) {
        return Ok(Outcome::Unchanged);
    }
    let dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.yeni", std::process::id()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map(|()| Outcome::Written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(root: &Path) -> SearchDirs {
        SearchDirs {
            config_home: root.join("config"),
            config_dirs: vec![root.join("xdg1"), root.join("xdg2")],
            sysconfdir: root.join("etc"),
            data_home: root.join("data"),
            data_dirs: vec![root.join("share1"), root.join("share2")],
            datadir: root.join("usr"),
        }
    }

    fn put(dir: &Path, name: &str, text: &str) -> PathBuf {
        let d = dir.join(SUBDIR);
        fs::create_dir_all(&d).unwrap();
        let p = d.join(name);
        fs::write(&p, text).unwrap();
        p
    }

    fn keys(text: &str) -> Vec<(String, String)> {
        load(text).expect("geçerli")
    }

    fn value(text: &str, key: &str) -> Option<String> {
        load(text)?.into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    #[test]
    fn search_order_config_home_first() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        put(&d.data_dirs[1], "hyprland-portals.conf", "[preferred]\ndefault=a\n");
        put(&d.data_home, "hyprland-portals.conf", "[preferred]\ndefault=b\n");
        put(&d.sysconfdir, "portals.conf", "[preferred]\ndefault=c\n");
        put(&d.config_dirs[1], "hyprland-portals.conf", "[preferred]\ndefault=d\n");
        let p = put(&d.config_home, "hyprland-portals.conf", "[preferred]\ndefault=e\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, p);
    }

    #[test]
    fn search_order_walks_tiers() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        let usr = put(&d.datadir, "hyprland-portals.conf", "[preferred]\ndefault=a\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, usr);
        let share2 = put(&d.data_dirs[1], "portals.conf", "[preferred]\ndefault=a\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, share2);
        let data = put(&d.data_home, "portals.conf", "[preferred]\ndefault=a\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, data);
        let etc = put(&d.sysconfdir, "hyprland-portals.conf", "[preferred]\ndefault=a\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, etc);
        let xdg2 = put(&d.config_dirs[1], "hyprland-portals.conf", "[preferred]\ndefault=a\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, xdg2);
        let xdg1 = put(&d.config_dirs[0], "portals.conf", "[preferred]\ndefault=a\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, xdg1);
    }

    #[test]
    fn desktop_file_before_generic_in_same_dir() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        put(&d.config_home, "portals.conf", "[preferred]\ndefault=gtk\n");
        let p = put(&d.config_home, "hyprland-portals.conf", "[preferred]\ndefault=kde\n");
        assert_eq!(find_source(&d, HOST_DESKTOPS).unwrap().path, p);
    }

    #[test]
    fn nuxut_file_and_invalid_files_skipped() {
        let t = tempfile::tempdir().unwrap();
        let d = dirs(t.path());
        put(&d.config_home, FILE_NAME, &render(None));
        // [preferred]'da anahtar yok / ayrıştırılamaz: xdp atlar.
        put(&d.config_home, "hyprland-portals.conf", "[preferred]\n# boş\n[other]\nx=y\n");
        put(&d.config_home, "portals.conf", "bozuk satır\n");
        let p = put(&d.config_dirs[0], "hyprland-portals.conf", "[preferred]\ndefault=gtk\n");
        assert_eq!(find_source(&d, &["nuxut", "hyprland"]).unwrap().path, p);
    }

    #[test]
    fn keeps_every_key_and_comment() {
        let src = "# kullanıcı yorumu\n[preferred]\ndefault = hyprland;gtk\n\
                   org.freedesktop.impl.portal.FileChooser = kde\nx.y.Unknown=foo;bar\n\n[extra]\nk=v\n";
        let out = render(Some(&Source { path: "/k/hyprland-portals.conf".into(), text: src.into() }));
        assert_eq!(out.lines().next(), Some(PORTALS_CONF_MARK));
        assert!(out.contains("# türetildi: /k/hyprland-portals.conf"));
        assert!(out.contains("# kullanıcı yorumu\n"));
        assert!(out.contains("org.freedesktop.impl.portal.FileChooser=nuxut;kde\n"), "dosya seçici devralındı");
        assert!(out.ends_with(&format!("[{BACKUP_GROUP}]\norg.freedesktop.impl.portal.FileChooser=kde\n")));
        assert!(out.contains("x.y.Unknown=foo;bar\n"));
        assert!(out.contains("[extra]\nk=v\n"));
        assert_eq!(value(&out, "org.freedesktop.impl.portal.FileChooser").as_deref(), Some("nuxut;kde"));
        assert_eq!(value(&out, SETTINGS_KEY).as_deref(), Some("nuxut;hyprland;gtk"));
        assert_eq!(value(&out, "default").as_deref(), Some("hyprland;gtk"));
        // Settings ilk [preferred] bölümünün son anahtarından sonra, [extra]'dan önce.
        let s = out.find(&format!("\n{SETTINGS_KEY}=")).unwrap();
        assert!(s > out.find("x.y.Unknown").unwrap() && s < out.find("[extra]").unwrap());
        // Ürettiğimiz dosya xdp'nin ayrıştırıcısına da geçerli.
        assert!(load(&out).is_some());
    }

    #[test]
    fn settings_merge() {
        let v = |t: &str| settings_value(&keys(t));
        // Settings var: o liste, default değil.
        assert_eq!(v("[preferred]\ndefault=hyprland;gtk\norg.freedesktop.impl.portal.Settings=kde\n"), "nuxut;kde");
        // Settings yok: default.
        assert_eq!(v("[preferred]\ndefault = hyprland ; gtk ;\n"), "nuxut;hyprland;gtk");
        // Zaten nuxut içeriyor: tekrar yok, sıra korunur.
        assert_eq!(v("[preferred]\norg.freedesktop.impl.portal.Settings=gtk;nuxut;gtk\n"), "nuxut;gtk");
        // none atılır (yoksa xdp arayüzü kapatır).
        assert_eq!(v("[preferred]\ndefault=none\n"), "nuxut");
        // Hiçbiri yok.
        assert_eq!(v("[preferred]\norg.freedesktop.impl.portal.FileChooser=kde\n"), "nuxut");
        // Yinelenen anahtarda sonuncusu (GKeyFile).
        assert_eq!(v("[preferred]\ndefault=a\n[preferred]\ndefault=b\n"), "nuxut;b");
    }

    #[test]
    fn settings_line_replaced_in_place_once() {
        let src = "[preferred]\norg.freedesktop.impl.portal.Settings=gtk\ndefault=hyprland\n\
                   [preferred]\norg.freedesktop.impl.portal.Settings=kde\n";
        let out = rewrite(src, false).unwrap();
        assert_eq!(out.matches(SETTINGS_KEY).count(), 1);
        assert!(out.starts_with("[preferred]\norg.freedesktop.impl.portal.Settings=nuxut;kde\ndefault=hyprland\n"));
    }

    /// Nuxut varsayılan (kullanıcı ilkesi 2026-10-16): açık seçim (ör. `kde`) de
    /// devralınır; önceki seçim yedek arka uç olur ve özgün satır xdp'nin okumadığı grupta saklanır.
    #[test]
    fn file_chooser_takes_over_with_backup() {
        let with = rewrite("[preferred]\ndefault=hyprland;gtk;none\n", true).unwrap();
        assert_eq!(value(&with, FILE_CHOOSER_KEY).as_deref(), Some("nuxut;hyprland;gtk"));
        assert_eq!(with.matches(FILE_CHOOSER_KEY).count(), 1);
        assert!(!with.contains(BACKUP_GROUP), "açık seçim yoksa yedek yok");
        assert!(load(&with).is_some());
        let src = "[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser = kde\nx.Other=a\n\
                   [preferred]\norg.freedesktop.impl.portal.FileChooser=kde;none\n";
        let user = rewrite(src, true).unwrap();
        assert_eq!(value(&user, FILE_CHOOSER_KEY).as_deref(), Some("nuxut;kde"), "sonuncu (GKeyFile), none atılır");
        // Yerinde tek satır: ilk geçtiği yerde, ikinci bölümdeki silinir.
        assert!(user.starts_with("[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser=nuxut;kde\nx.Other=a\n"));
        let (preferred, backup) = user.split_once(&format!("[{BACKUP_GROUP}]")).unwrap();
        assert_eq!(preferred.matches(FILE_CHOOSER_KEY).count(), 1);
        assert_eq!(backup, format!("\n{FILE_CHOOSER_KEY}=kde;none\n"), "özgün değer yedekte");
        // GKeyFile geçerli ve xdp'nin okuduğu [preferred] yalnız devralınan değeri görür.
        assert!(parse(&user).is_some());
        // Bu makinenin düzeni: [preferred]'ın son anahtarı FileChooser; Settings yine eklenir.
        let here = rewrite("[preferred]\ndefault = hyprland;gtk\norg.freedesktop.impl.portal.FileChooser = kde\n", true).unwrap();
        assert!(here.starts_with(
            "[preferred]\ndefault = hyprland;gtk\norg.freedesktop.impl.portal.FileChooser=nuxut;kde\norg.freedesktop.impl.portal.Settings=nuxut;hyprland;gtk\n"
        ));
        assert_eq!(value(&here, SETTINGS_KEY).as_deref(), Some("nuxut;hyprland;gtk"));
        // Zaten Nuxut ise yedek yok, satır aynı kalır.
        let ours = rewrite("[preferred]\norg.freedesktop.impl.portal.FileChooser=nuxut;kde\n", true).unwrap();
        assert!(!ours.contains(BACKUP_GROUP));
        assert_eq!(value(&ours, FILE_CHOOSER_KEY).as_deref(), Some("nuxut;kde"));
        // Kapalı (--no-filechooser): kaynak olduğu gibi (açık seçim geçerli), eklenmez.
        let off = rewrite("[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser=kde\n", false).unwrap();
        assert_eq!(value(&off, FILE_CHOOSER_KEY).as_deref(), Some("kde"));
        assert_eq!(value(&rewrite("[preferred]\ndefault=gtk\n", false).unwrap(), FILE_CHOOSER_KEY), None);
    }

    /// `--no-filechooser` dosyada işaret bırakır (yeniden üretim kapalı tutar), geri alma ve
    /// yeniden açma komutları başlıkta.
    #[test]
    fn file_chooser_opt_out_is_sticky_and_documented() {
        let src = Source { path: "/k/hyprland-portals.conf".into(), text: "[preferred]\nFoo=1\norg.freedesktop.impl.portal.FileChooser=kde\n".into() };
        let on = render_opts(Some(&src), "/l/x --write-portals-conf", true);
        assert!(!file_chooser_disabled(&on));
        assert!(on.contains("# Geri almak (önceki seçime dönmek) için:  /l/x --write-portals-conf --no-filechooser\n"));
        let off = render_opts(Some(&src), "/l/x --write-portals-conf --no-filechooser", false);
        assert!(file_chooser_disabled(&off));
        assert!(off.contains("Nuxut seçicisi için:  /l/x --write-portals-conf --filechooser\n"));
        assert!(off.contains("#   /l/x --write-portals-conf   (ya da install.sh)\n"));
        assert_eq!(value(&off, FILE_CHOOSER_KEY).as_deref(), Some("kde"));
        assert_eq!(off.lines().next(), Some(PORTALS_CONF_MARK), "işaret ilk satırda kalır");
    }

    #[test]
    fn no_source_defaults() {
        let out = render(None);
        assert_eq!(out.lines().next(), Some(PORTALS_CONF_MARK));
        assert!(out.contains("# kaynak yok"));
        assert_eq!(value(&out, "default").as_deref(), Some("hyprland;gtk"));
        assert_eq!(value(&out, SETTINGS_KEY).as_deref(), Some("nuxut;hyprland;gtk"));
        assert_eq!(value(&out, FILE_CHOOSER_KEY).as_deref(), Some("nuxut;hyprland;gtk"));
        assert_eq!(value(&render_opts(None, DEFAULT_REGEN, false), FILE_CHOOSER_KEY), None);
    }

    #[test]
    fn from_vars_glib_defaults() {
        let d = SearchDirs::from_vars(|k| match k {
            "HOME" => Some("/h".into()),
            "XDG_CONFIG_HOME" => Some("".into()),
            "XDG_DATA_DIRS" => Some("/a::/b".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(d.config_home, Path::new("/h/.config"));
        assert_eq!(d.data_home, Path::new("/h/.local/share"));
        assert_eq!(d.config_dirs, vec![PathBuf::from("/etc/xdg")]);
        assert_eq!(d.data_dirs, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(d.target(), Path::new("/h/.config/xdg-desktop-portal/nuxut-portals.conf"));
        assert_eq!(d.ordered()[2], Path::new("/etc/xdg-desktop-portal"));
        assert_eq!(d.ordered().last().unwrap(), Path::new("/usr/share/xdg-desktop-portal"));
    }

    #[test]
    fn write_creates_replaces_ours_and_respects_user() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("c/xdg-desktop-portal").join(FILE_NAME);
        let a = render(None);
        assert_eq!(write(&target, &a).unwrap(), Outcome::Written);
        assert_eq!(fs::read_to_string(&target).unwrap(), a);
        assert_eq!(write(&target, &a).unwrap(), Outcome::Unchanged);
        let b = render(Some(&Source { path: "/x".into(), text: "[preferred]\ndefault=kde\n".into() }));
        assert_eq!(write(&target, &b).unwrap(), Outcome::Written);
        assert_eq!(fs::read_to_string(&target).unwrap(), b);
        // Geçici dosya kalmadı.
        let left: Vec<_> = fs::read_dir(target.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1);
        // İşaret silindi: kullanıcının; dokunulmaz.
        let user = "[preferred]\ndefault=gtk\n";
        fs::write(&target, user).unwrap();
        assert_eq!(write(&target, &a).unwrap(), Outcome::UserOwned);
        assert_eq!(fs::read_to_string(&target).unwrap(), user);
        assert!(!is_ours(&target).unwrap());
    }

    #[test]
    fn mark_matches_install_sh() {
        assert!(PORTALS_CONF_MARK.starts_with(PORTALS_CONF_PREFIX));
        let sh = include_str!("../../install.sh");
        assert!(sh.contains(&format!("PORTALS_CONF_PREFIX=\"{PORTALS_CONF_PREFIX}\"\n")));
        assert!(sh.contains("case \"$(head -n 1 \"$PORTALS_CONF\")\" in \"$PORTALS_CONF_PREFIX\"*)"));
    }

    /// Aynı önek, farklı açıklama (eski/yeni sürümün işareti) → Nuxut'un; önek yoksa kullanıcının.
    #[test]
    fn ownership_by_prefix() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join(FILE_NAME);
        for (first, ours) in [
            (format!("{PORTALS_CONF_PREFIX} — eski açıklama"), true),
            (PORTALS_CONF_PREFIX.to_owned(), true),
            (PORTALS_CONF_MARK.to_owned(), true),
            ("# nuxut-managed: portal".to_owned(), false),
            (format!(" {PORTALS_CONF_PREFIX}"), false),
            ("[preferred]".to_owned(), false),
        ] {
            fs::write(&target, format!("{first}\n[preferred]\ndefault=gtk\n")).unwrap();
            assert_eq!(is_ours(&target).unwrap(), ours, "{first}");
        }
        fs::write(&target, format!("{PORTALS_CONF_PREFIX} — eski\n")).unwrap();
        assert_eq!(write(&target, &render(None)).unwrap(), Outcome::Written);
    }

    #[test]
    fn write_is_atomic_rename() {
        // rename aynı inode'u yerinde değiştirmez: eski dosyayı açık tutan okuyucu eski içeriği görür.
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join(FILE_NAME);
        write(&target, &render(None)).unwrap();
        let old = fs::File::open(&target).unwrap();
        use std::os::unix::fs::MetadataExt as _;
        let ino = old.metadata().unwrap().ino();
        let b = render(Some(&Source { path: "/x".into(), text: "[preferred]\ndefault=kde\n".into() }));
        write(&target, &b).unwrap();
        assert_ne!(fs::metadata(&target).unwrap().ino(), ino);
        let mut s = String::new();
        io::Read::read_to_string(&mut &old, &mut s).unwrap();
        assert_eq!(s, render(None));
    }
}

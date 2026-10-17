//! Ekran paylaşım seçicisinin xdg-desktop-portal-hyprland (xdph) sözleşmesi: UI'dan bağımsız,
//! saf ve test edilebilir. Sözleşmenin kaynağı ve doğrulanmamış noktaları:
//! `docs/design/screenshare-picker.md`.
//!
//! - **Girdi:** xdph seçiciyi `screencopy:custom_picker_binary` ile başlatır; argüman yalnızca
//!   `--allow-token` (`screencopy:allow_token_by_default` açıksa). Pencere listesi
//!   `XDPH_WINDOW_SHARING_LIST` ortam değişkeninde:
//!   `<id>[HC>]<sınıf>[HT>]<başlık>[HE>]<adres>[HA>]` girdilerinin art arda dizilişi. `id` toplevel
//!   tutamacının alt 32 biti (ondalık), `adres` Hyprland pencere adresi (ondalık u64). xdph sınıf
//!   ve başlıkta `' " $ \`` karakterlerini boşluğa, `>]` dizisindeki `]`'yi boşluğa çevirir; bu
//!   yüzden işaretler metnin içinde geçemez ve bölme tek anlamlıdır.
//! - **Çıktı:** stdout'ta tek satır `[SELECTION]<bayraklar>/<seçim>\n`. Bayrak `r`: uygulamaya
//!   geri yükleme belirteci (restore token) verilsin. Seçim `screen:<çıkış>`,
//!   `window:<id>` ya da `region:<çıkış>@<x>,<y>,<genişlik>,<yükseklik>` (çıkışa göre yerel,
//!   mantıksal piksel).
//! - **İptal:** stdout'ta `[SELECTION]` yoksa xdph isteği iptal sayar.
//!
//! Dış girdi (ortam değişkeni, compositor'ın çıkış adları) boyut sınırlı ve doğrulanır;
//! hiçbir yol panik etmez.

use std::fmt;

/// Pencere listesi ortam değişkeninin adı.
pub const WINDOW_LIST_ENV: &str = "XDPH_WINDOW_SHARING_LIST";
/// xdph'in `screencopy:allow_token_by_default` açıkken verdiği argüman.
pub const ALLOW_TOKEN_ARG: &str = "--allow-token";
/// Çıktı satırının ön eki (xdph bunu arar).
pub const SELECTION_PREFIX: &str = "[SELECTION]";
/// Geri yükleme belirteci bayrağı.
pub const RESTORE_FLAG: char = 'r';

/// Pencere listesinin en uzun hâli (bayt). Linux'ta tek bir ortam dizgesi zaten
/// `MAX_ARG_STRLEN` (128 KiB) ile sınırlı; daha uzunu bozuk sayılır ve tümden reddedilir.
pub const MAX_LIST_BYTES: usize = 128 * 1024;
/// Listeden alınan en çok pencere (fazlası atlanır, `truncated`).
pub const MAX_WINDOWS: usize = 256;
/// Gösterilen sınıf ve başlığın en uzun hâli (karakter; fazlası `…` ile kırpılır).
pub const MAX_CLASS_CHARS: usize = 96;
pub const MAX_TITLE_CHARS: usize = 200;
/// Çıkış adının en uzun hâli (bayt).
pub const MAX_OUTPUT_NAME: usize = 64;

const MARK_CLASS: &str = "[HC>]";
const MARK_TITLE: &str = "[HT>]";
const MARK_ADDRESS: &str = "[HE>]";
const MARK_END: &str = "[HA>]";

/// Listedeki paylaşılabilir pencere. `class`/`title` gösterime hazırdır (temizlenmiş, kırpılmış).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedWindow {
    /// xdph'e geri verilen kimlik (toplevel tutamacının alt 32 biti).
    pub id: u32,
    pub class: String,
    pub title: String,
    /// Hyprland pencere adresi; ayrıştırılamazsa 0 (yalnızca bilgi, çıktıya girmez).
    pub address: u64,
}

/// Ayrıştırılmış pencere listesi.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowList {
    pub windows: Vec<SharedWindow>,
    /// Atlanan bozuk girdi sayısı (eksik işaret, geçersiz kimlik, yinelenen kimlik, yarım girdi).
    pub malformed: usize,
    /// [`MAX_WINDOWS`] aşıldı ya da liste [`MAX_LIST_BYTES`]'tan uzundu.
    pub truncated: bool,
}

impl WindowList {
    /// `XDPH_WINDOW_SHARING_LIST` değerini ayrıştırır. Panik etmez; bozuk girdi atlanır.
    pub fn parse(raw: &str) -> Self {
        let mut out = Self::default();
        if raw.len() > MAX_LIST_BYTES {
            out.truncated = true;
            return out;
        }
        let mut pieces = raw.split(MARK_END).peekable();
        while let Some(piece) = pieces.next() {
            if pieces.peek().is_none() {
                // Son `[HA>]`'den sonrası: boş olmalı (yalnızca boşluk/satır sonu kabul).
                if !piece.trim().is_empty() {
                    out.malformed += 1;
                }
                break;
            }
            if out.windows.len() >= MAX_WINDOWS {
                out.truncated = true;
                break;
            }
            match parse_entry(piece) {
                Some(w) if out.windows.iter().any(|o| o.id == w.id) => out.malformed += 1,
                Some(w) => out.windows.push(w),
                None => out.malformed += 1,
            }
        }
        out
    }
}

fn parse_entry(piece: &str) -> Option<SharedWindow> {
    let (id, rest) = piece.split_once(MARK_CLASS)?;
    let (class, rest) = rest.split_once(MARK_TITLE)?;
    let (title, address) = rest.split_once(MARK_ADDRESS)?;
    let id = parse_decimal_u32(id.trim())?;
    let address = address.trim().parse::<u64>().unwrap_or(0);
    Some(SharedWindow {
        id,
        class: display_text(class, MAX_CLASS_CHARS),
        title: display_text(title, MAX_TITLE_CHARS),
        address,
    })
}

/// Yalnızca ASCII rakamlar (işaret, boşluk, `0x` yok); u32 taşması reddedilir.
fn parse_decimal_u32(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Gösterim metni: denetim, yön (bidi) ve sıfır genişlikli karakterler atılır, boşluklar
/// teklenir, `max_chars`'tan uzunsa sonu `…` ile kırpılır.
pub fn display_text(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 4));
    let mut count = 0usize;
    let mut pending_space = false;
    for c in s.chars() {
        // Önce boşluk: sekme ve satır sonu da denetim karakteridir, boşluğa döner.
        if c.is_whitespace() {
            pending_space = count > 0;
            continue;
        }
        if is_invisible(c) {
            continue;
        }
        let need = if pending_space { 2 } else { 1 };
        if count + need > max_chars {
            if max_chars > 0 {
                // Son karakterin yerine üç nokta (toplam `max_chars`).
                if count >= max_chars {
                    out.pop();
                }
                out.push('…');
            }
            return out;
        }
        if pending_space {
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        out.push(c);
        count += 1;
    }
    out
}

fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Çıkış adı sözleşmeye yazılabilir mi: boş değil, [`MAX_OUTPUT_NAME`] baytı aşmıyor; denetim
/// karakteri, boşluk ve sözleşmenin ayırıcıları (`@ / ,`) yok. (xdph `screen:` adını satır
/// sonuna kadar, `region:` adını ilk `@`'ye kadar okur.)
pub fn valid_output_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_OUTPUT_NAME
        && name.chars().all(|c| !c.is_control() && !c.is_whitespace() && !is_invisible(c) && !matches!(c, '@' | '/' | ','))
}

/// Seçilen bölge: çıkışa göre yerel, mantıksal piksel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Region {
    /// Kayan noktalı dikdörtgeni (`x, y, genişlik, yükseklik`) çıkış sınırına (`bounds` = çıkışın mantıksal
    /// boyutu) kırpıp kenarları yuvarlar. Boş ya da sonlu olmayan girdi `None`.
    pub fn from_logical(output: &str, rect: (f32, f32, f32, f32), bounds: (f32, f32)) -> Option<Self> {
        let (x, y, w, h) = rect;
        let (bw, bh) = bounds;
        if ![x, y, w, h, bw, bh].iter().all(|v| v.is_finite()) || bw < 1.0 || bh < 1.0 {
            return None;
        }
        let max = i32::MAX as f32;
        let bw = bw.round().min(max);
        let bh = bh.round().min(max);
        let x0 = x.round().clamp(0.0, bw);
        let y0 = y.round().clamp(0.0, bh);
        let x1 = (x + w).round().clamp(0.0, bw);
        let y1 = (y + h).round().clamp(0.0, bh);
        if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
            return None;
        }
        // Değerler [0, i32::MAX] aralığında kırpıldı: dönüşüm kayıpsız ve taşmasız.
        Some(Self {
            output: output.to_owned(),
            x: x0 as i32,
            y: y0 as i32,
            width: (x1 - x0) as i32,
            height: (y1 - y0) as i32,
        })
    }
}

/// Kullanıcının seçimi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Çıkışın tamamı (çıkış adı).
    Output(String),
    /// Listedeki pencere kimliği.
    Window(u32),
    Region(Region),
}

/// Seçim sözleşmeye yazılamıyor (geçersiz çıkış adı ya da bölge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidChoice;

impl fmt::Display for InvalidChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("seçim sözleşmeye yazılamıyor (geçersiz çıkış adı ya da bölge)")
    }
}

impl std::error::Error for InvalidChoice {}

/// stdout'a yazılacak tek satır (sonunda `\n`).
pub fn selection_line(choice: &Choice, allow_token: bool) -> Result<String, InvalidChoice> {
    let flags = if allow_token { RESTORE_FLAG.to_string() } else { String::new() };
    let body = match choice {
        Choice::Output(name) if valid_output_name(name) => format!("screen:{name}"),
        Choice::Window(id) => format!("window:{id}"),
        Choice::Region(r)
            if valid_output_name(&r.output) && r.x >= 0 && r.y >= 0 && r.width > 0 && r.height > 0 =>
        {
            format!("region:{}@{},{},{},{}", r.output, r.x, r.y, r.width, r.height)
        }
        _ => return Err(InvalidChoice),
    };
    Ok(format!("{SELECTION_PREFIX}{flags}/{body}\n"))
}

/// Komut satırı.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PickerArgs {
    /// "İzni hatırla" başlangıçta açık (`--allow-token`).
    pub allow_token: bool,
    /// Tanınmayan argümanlar (ileri uyumluluk: yok sayılır, loglanır).
    pub unknown: Vec<String>,
}

impl PickerArgs {
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Self {
        let mut out = Self::default();
        for a in args {
            if a == ALLOW_TOKEN_ARG {
                out.allow_token = true;
            } else if out.unknown.len() < 16 {
                out.unknown.push(display_text(&a, 64));
            }
        }
        out
    }
}

/// Oturum kilidine göre seçicinin ne yapabileceği (logind `LockedHint`; 2026-10-17).
///
/// **Kapalı-güvenli (2026-10-17, güvenlik incelemesi):** yalnız logind'den okunmuş, oturumumuza ait
/// `LockedHint = false` kilitsiz sayılır. Kilitli, bilinmeyen ya da okunamayan durum (logind yok,
/// oturum çözülemedi, D-Bus hatası, bağlantı koptu, [`LOCK_DEADLINE`] içinde yanıt yok) kilitli
/// kabul edilir: istek reddedilir (seçim yazılmadan çıkılır, xdph iptal sayar) ve hiçbir pencere
/// yakalanmaz. İlk yanıt beklenirken önizleme istenmez, "Paylaş" işlemez.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LockGate {
    #[default]
    Unknown,
    Unlocked,
    Locked,
}

/// İlk logind yanıtı için en uzun bekleme; dolarsa durum bilinmiyor = kilitli.
pub const LOCK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

impl LockGate {
    /// logind anlık görüntüsünden kapı durumu: yalnız okunmuş "kilitsiz" açar.
    pub fn from_logind(available: bool, has_session: bool, locked_hint: bool) -> Self {
        if available && has_session && !locked_hint {
            Self::Unlocked
        } else {
            Self::Locked
        }
    }

    /// [`LOCK_DEADLINE`] doldu: hâlâ bilinmiyorsa kilitli.
    pub fn after_deadline(self) -> Self {
        match self {
            Self::Unknown => Self::Locked,
            other => other,
        }
    }

    /// Pencere önizlemesi yakalanabilir.
    pub fn previews_allowed(self) -> bool {
        self == Self::Unlocked
    }

    /// Seçim kabul edilebilir.
    pub fn share_allowed(self) -> bool {
        self == Self::Unlocked
    }

    /// İstek reddedilmeli (seçici kapanır).
    pub fn must_reject(self) -> bool {
        self == Self::Locked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_gate_fails_closed_when_locked_unknown_or_unreadable() {
        let unknown = LockGate::default();
        assert!(!unknown.previews_allowed() && !unknown.share_allowed() && !unknown.must_reject());
        let locked = LockGate::from_logind(true, true, true);
        assert_eq!(locked, LockGate::Locked);
        assert!(!locked.previews_allowed() && !locked.share_allowed() && locked.must_reject());
        let open = LockGate::from_logind(true, true, false);
        assert!(open.previews_allowed() && open.share_allowed() && !open.must_reject());
        // Kapalı-güvenli: logind yok, oturum çözülemedi, okunamadı → kilitli, reddet.
        assert_eq!(LockGate::from_logind(false, false, false), LockGate::Locked);
        assert_eq!(LockGate::from_logind(false, true, false), LockGate::Locked, "logind düştü");
        assert_eq!(LockGate::from_logind(true, false, false), LockGate::Locked, "oturum yok");
        // Yanıt gelmedi (zaman aşımı): bilinmiyor → kilitli; bilinen durum değişmez.
        assert!(LockGate::Unknown.after_deadline().must_reject());
        assert_eq!(LockGate::Unlocked.after_deadline(), LockGate::Unlocked);
        assert_eq!(LockGate::from_logind(true, false, true), LockGate::Locked);
    }

    fn entry(id: &str, class: &str, title: &str, addr: &str) -> String {
        format!("{id}[HC>]{class}[HT>]{title}[HE>]{addr}[HA>]")
    }

    #[test]
    fn parses_xdph_window_list() {
        let raw = [entry("3045123456", "firefox", "Nuxut — Mozilla Firefox", "94823749823"), entry("17", "kitty", "~", "0")].concat();
        let list = WindowList::parse(&raw);
        assert_eq!(list.malformed, 0);
        assert!(!list.truncated);
        assert_eq!(
            list.windows,
            vec![
                SharedWindow { id: 3_045_123_456, class: "firefox".into(), title: "Nuxut — Mozilla Firefox".into(), address: 94_823_749_823 },
                SharedWindow { id: 17, class: "kitty".into(), title: "~".into(), address: 0 },
            ]
        );
    }

    #[test]
    fn empty_and_missing_lists() {
        assert_eq!(WindowList::parse(""), WindowList::default());
        // Sonda satır sonu zararsız.
        let list = WindowList::parse(&format!("{}\n", entry("1", "a", "b", "2")));
        assert_eq!((list.windows.len(), list.malformed), (1, 0));
    }

    #[test]
    fn malformed_entries_are_skipped() {
        let raw = [
            entry("abc", "x", "y", "1"),           // kimlik sayı değil
            entry("-5", "x", "y", "1"),            // işaret yok
            entry("4294967296", "x", "y", "1"),    // u32 taşması
            "9[HC>]eksik[HA>]".to_string(),        // işaret eksik
            entry("7", "ok", "pencere", "x"),      // adres bozuk → 0, girdi geçerli
            entry("7", "dup", "aynı kimlik", "1"), // yinelenen kimlik
            "8[HC>]a[HT>]b[HE>]1".to_string(),     // yarım (sonda [HA>] yok)
        ]
        .concat();
        let list = WindowList::parse(&raw);
        assert_eq!(list.windows, vec![SharedWindow { id: 7, class: "ok".into(), title: "pencere".into(), address: 0 }]);
        assert_eq!(list.malformed, 6);
    }

    #[test]
    fn oversized_list_rejected_without_panic() {
        let one = entry("1", &"c".repeat(100), &"t".repeat(1000), "1");
        let raw = one.repeat(MAX_LIST_BYTES / one.len() + 2);
        assert!(raw.len() > MAX_LIST_BYTES);
        let list = WindowList::parse(&raw);
        assert!(list.truncated && list.windows.is_empty());
    }

    #[test]
    fn window_count_capped() {
        let raw: String = (0..MAX_WINDOWS as u32 + 10).map(|i| entry(&i.to_string(), "c", "t", "0")).collect();
        let list = WindowList::parse(&raw);
        assert_eq!(list.windows.len(), MAX_WINDOWS);
        assert!(list.truncated);
    }

    #[test]
    fn display_text_strips_and_clips() {
        assert_eq!(display_text("a\u{202E}b\u{200B}c\n d\t\te", 50), "abc d e");
        assert_eq!(display_text("   baş  ", 50), "baş");
        assert_eq!(display_text("abcdef", 4), "abc…");
        assert_eq!(display_text("abcd", 4), "abcd");
        assert_eq!(display_text("ab cd", 3), "ab…");
        assert_eq!(display_text("ğüşıöç", 3), "ğü…");
        assert_eq!(display_text("abc", 0), "");
        let long = "é".repeat(10_000);
        assert_eq!(display_text(&long, MAX_TITLE_CHARS).chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn titles_with_marker_lookalikes() {
        // xdph `>]` içindeki `]`'yi boşluğa çevirir; yarım işaretler metinde kalır.
        let raw = entry("5", "c", "[HA> başlık [HC> x", "1");
        let list = WindowList::parse(&raw);
        assert_eq!(list.windows.len(), 1);
        assert_eq!(list.windows[0].title, "[HA> başlık [HC> x");
    }

    #[test]
    fn output_names() {
        for ok in ["DP-1", "HDMI-A-1", "eDP-1", "HEADLESS-1", "Virtual-1"] {
            assert!(valid_output_name(ok), "{ok}");
        }
        for bad in ["", "DP 1", "DP-1\n", "a@b", "a/b", "a,b", "x\u{202E}", &"A".repeat(65)] {
            assert!(!valid_output_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn selection_lines() {
        assert_eq!(selection_line(&Choice::Output("DP-1".into()), false).unwrap(), "[SELECTION]/screen:DP-1\n");
        assert_eq!(selection_line(&Choice::Output("DP-1".into()), true).unwrap(), "[SELECTION]r/screen:DP-1\n");
        assert_eq!(selection_line(&Choice::Window(3_045_123_456), false).unwrap(), "[SELECTION]/window:3045123456\n");
        let r = Region { output: "HDMI-A-1".into(), x: 10, y: 20, width: 300, height: 200 };
        assert_eq!(selection_line(&Choice::Region(r), true).unwrap(), "[SELECTION]r/region:HDMI-A-1@10,20,300,200\n");
        assert_eq!(selection_line(&Choice::Output("a@b".into()), false), Err(InvalidChoice));
        let empty = Region { output: "DP-1".into(), x: 0, y: 0, width: 0, height: 5 };
        assert_eq!(selection_line(&Choice::Region(empty), false), Err(InvalidChoice));
    }

    #[test]
    fn region_from_logical_rounds_and_clips() {
        let r = Region::from_logical("DP-1", (10.4, 20.6, 100.2, 50.0), (1920.0, 1080.0)).unwrap();
        // Kenarlar yuvarlanır: 10.4 → 10, 110.6 → 111.
        assert_eq!((r.x, r.y, r.width, r.height), (10, 21, 101, 50));
        // Sınır dışı kırpılır.
        let r = Region::from_logical("DP-1", (-50.0, 1000.0, 200.0, 500.0), (1920.0, 1080.0)).unwrap();
        assert_eq!((r.x, r.y, r.width, r.height), (0, 1000, 150, 80));
        assert_eq!(Region::from_logical("DP-1", (5.0, 5.0, 0.2, 10.0), (100.0, 100.0)), None);
        assert_eq!(Region::from_logical("DP-1", (f32::NAN, 0.0, 10.0, 10.0), (100.0, 100.0)), None);
        assert_eq!(Region::from_logical("DP-1", (0.0, 0.0, f32::INFINITY, 10.0), (100.0, 100.0)), None);
        assert_eq!(Region::from_logical("DP-1", (0.0, 0.0, 10.0, 10.0), (0.0, 100.0)), None);
        let r = Region::from_logical("DP-1", (0.0, 0.0, 1e30, 1e30), (1e30, 1e30)).unwrap();
        assert_eq!((r.x, r.y), (0, 0));
        assert!(r.width > 0 && r.height > 0);
    }

    #[test]
    fn args() {
        let a = PickerArgs::parse(["--allow-token".to_string(), "--yeni".to_string()]);
        assert!(a.allow_token);
        assert_eq!(a.unknown, vec!["--yeni".to_string()]);
        assert_eq!(PickerArgs::parse(Vec::new()), PickerArgs::default());
    }
}

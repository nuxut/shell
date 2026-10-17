//! Gömülü xkb keymap'leri ve tuş etiketleri.
//!
//! Keymap metinleri derleme içidir (`assets/osk/<düzen>.xkb`): kullanıcının sistemindeki xkb
//! verisine (yerel değişiklikler, `XKB_DEFAULT_OPTIONS`) bağlı değildir; sanal klavyeye her zaman
//! aynı metin gönderilir. Kaynak: değiştirilmemiş xkeyboard-config 2.48 (Arch paketi), `evdev`
//! kuralları, `pc105` modeli, seçeneksiz; libxkbcommon 1.13 ile `--output-format 1
//! --drop-unused` (yeniden üretim komutu `docs/design/osk.md`).
//!
//! Etiketler aynı keymap'ten xkbcommon ile çıkarılır ([`Labels`]): ekranda görünen karakter,
//! istemcinin aynı tuş ve değiştiricilerle üreteceği karakterin ta kendisidir (elle yazılmış
//! ikinci bir tablo yok).

use std::collections::HashMap;

use xkbcommon::xkb;

use super::keys;

/// Ekran klavyesi düzeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Layout {
    /// Türkçe Q (xkb `tr`).
    #[default]
    Tr,
    /// ABD İngilizcesi (xkb `us`).
    Us,
}

impl Layout {
    pub const ALL: [Layout; 2] = [Layout::Tr, Layout::Us];

    /// Ayar/IPC adı.
    pub const fn name(self) -> &'static str {
        match self {
            Layout::Tr => "tr",
            Layout::Us => "us",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == name)
    }

    /// Gömülü keymap metni (XKB_V1).
    pub const fn keymap(self) -> &'static str {
        match self {
            Layout::Tr => include_str!("../../../assets/osk/tr.xkb"),
            Layout::Us => include_str!("../../../assets/osk/us.xkb"),
        }
    }

    /// Sıradaki düzen (düzen tuşu).
    pub fn next(self) -> Self {
        match self {
            Layout::Tr => Layout::Us,
            Layout::Us => Layout::Tr,
        }
    }

    /// Compositor'ın bildirdiği etkin düzen adından (xkb grup adı: "Turkish", "English (US)").
    /// Türkçe Q dışındaki her şey US'e düşer (F klavye ayrı düzen, v1'de yok).
    pub fn from_xkb_name(name: &str) -> Self {
        let n = name.trim();
        if n == "Turkish" || n.starts_with("Turkish (") && !n.contains("(F)") {
            Layout::Tr
        } else {
            Layout::Us
        }
    }
}

/// Değiştirici maskeleri (keymap'in gerçek mod bitleri).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Masks {
    pub shift: u32,
    pub lock: u32,
    pub ctrl: u32,
    /// ISO_Level3_Shift (AltGr).
    pub level3: u32,
}

/// Etiket aranan değiştirici bileşimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Level {
    pub shift: bool,
    pub caps: bool,
    pub altgr: bool,
}

impl Level {
    const fn index(self) -> usize {
        (self.shift as usize) | (self.caps as usize) << 1 | (self.altgr as usize) << 2
    }

    fn from_index(i: usize) -> Self {
        Self { shift: i & 1 != 0, caps: i & 2 != 0, altgr: i & 4 != 0 }
    }
}

/// Keymap derlenemedi.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0} keymap'i derlenemedi")]
pub struct KeymapError(&'static str);

/// Bir düzenin derlenmiş etiket tablosu: her karakter tuşu için 8 değiştirici bileşiminde
/// görünen metin (`None` = o seviyede sembol yok; tuş o anda etkisiz).
#[derive(Debug, Clone)]
pub struct Labels {
    layout: Layout,
    masks: Masks,
    table: HashMap<u32, [Option<String>; 8]>,
    has_level3: bool,
}

impl Labels {
    /// Gömülü keymap'i xkbcommon ile derler ve tabloyu çıkarır.
    pub fn compile(layout: Layout) -> Result<Self, KeymapError> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            layout.keymap().to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or(KeymapError(layout.name()))?;
        let bit = |name: &str| {
            let i = keymap.mod_get_index(name);
            if i == xkb::MOD_INVALID { 0 } else { 1u32 << i }
        };
        let masks = Masks {
            shift: bit(xkb::MOD_NAME_SHIFT),
            lock: bit(xkb::MOD_NAME_CAPS),
            ctrl: bit(xkb::MOD_NAME_CTRL),
            level3: bit(xkb::MOD_NAME_ISO_LEVEL3_SHIFT),
        };
        if masks.shift == 0 || masks.lock == 0 || masks.ctrl == 0 {
            return Err(KeymapError(layout.name()));
        }
        let mut state = xkb::State::new(&keymap);
        let mut table = HashMap::new();
        let mut has_level3 = false;
        for code in keys::char_codes() {
            let mut row: [Option<String>; 8] = Default::default();
            for (i, slot) in row.iter_mut().enumerate() {
                let level = Level::from_index(i);
                let depressed = if level.shift { masks.shift } else { 0 } | if level.altgr { masks.level3 } else { 0 };
                let locked = if level.caps { masks.lock } else { 0 };
                state.update_mask(depressed, 0, locked, 0, 0, 0);
                *slot = label_of(&state, code);
            }
            // AltGr seviyesi düzende anlamlı mı: tuşun 3. seviyesi 1. seviyesinden farklı bir şey üretiyorsa.
            if masks.level3 != 0 && row[4].is_some() && row[4] != row[0] {
                has_level3 = true;
            }
            table.insert(code, row);
        }
        Ok(Self { layout, masks, table, has_level3 })
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn masks(&self) -> Masks {
        self.masks
    }

    /// Düzende AltGr (3. seviye) karakteri var mı (yoksa AltGr tuşu gösterilmez).
    pub fn has_level3(&self) -> bool {
        self.has_level3
    }

    /// Tuşun bu bileşimdeki etiketi.
    pub fn label(&self, code: u32, level: Level) -> Option<&str> {
        self.table.get(&code)?[level.index()].as_deref()
    }
}

/// Tuşun durumdaki metni; ölü tuşlar boşluklu karşılığıyla, sembolsüz seviye `None`.
fn label_of(state: &xkb::State, code: u32) -> Option<String> {
    let keycode = xkb::Keycode::new(code + 8);
    let sym = state.key_get_one_sym(keycode);
    if sym.raw() == 0 || sym == xkb::Keysym::VoidSymbol {
        return None;
    }
    let text = state.key_get_utf8(keycode);
    if !text.is_empty() && !text.chars().any(char::is_control) {
        return Some(text);
    }
    dead_key_label(&xkb::keysym_get_name(sym)).map(str::to_owned)
}

/// Ölü tuşun (`dead_*`) ekranda gösterilen boşluklu işareti.
fn dead_key_label(name: &str) -> Option<&'static str> {
    Some(match name.strip_prefix("dead_")? {
        "grave" => "`",
        "acute" => "´",
        "circumflex" => "^",
        "tilde" => "~",
        "macron" => "¯",
        "breve" => "˘",
        "abovedot" => "˙",
        "diaeresis" => "¨",
        "abovering" => "˚",
        "doubleacute" => "˝",
        "caron" => "ˇ",
        "cedilla" => "¸",
        "ogonek" => "˛",
        "hook" => "̉",
        "horn" => "̛",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::keys::code;

    const PLAIN: Level = Level { shift: false, caps: false, altgr: false };
    const SHIFT: Level = Level { shift: true, caps: false, altgr: false };
    const CAPS: Level = Level { shift: false, caps: true, altgr: false };
    const ALTGR: Level = Level { shift: false, caps: false, altgr: true };
    const SHIFT_ALTGR: Level = Level { shift: true, caps: false, altgr: true };

    fn tr() -> Labels {
        Labels::compile(Layout::Tr).expect("tr derlenir")
    }

    #[test]
    fn both_keymaps_compile_with_standard_masks() {
        for l in Layout::ALL {
            let labels = Labels::compile(l).unwrap();
            let m = labels.masks();
            // xkeyboard-config'in gerçek mod sırası: Shift, Lock, Control, Mod1..Mod5.
            assert_eq!((m.shift, m.lock, m.ctrl), (1, 2, 4), "{l:?}");
            assert_eq!(m.level3, 1 << 7, "{l:?} Mod5");
        }
    }

    #[test]
    fn embedded_keymaps_are_stock_not_local_overrides() {
        // Makinedeki değiştirilmiş xkb verisinden (Caps → Super) üretilmediğinin bekçisi.
        for l in Layout::ALL {
            let k = l.keymap();
            assert!(k.starts_with("xkb_keymap {"), "{l:?}");
            assert!(k.contains("modifier_map Lock { <CAPS> };"), "{l:?}: Caps kilit tuşu");
            assert!(k.contains("key <CAPS>               {\t[       Caps_Lock ] };"), "{l:?}");
        }
    }

    #[test]
    fn turkish_q_letters_and_shift() {
        let t = tr();
        let row: Vec<&str> = (code::AD01..=code::AD12).map(|c| t.label(c, PLAIN).unwrap()).collect();
        assert_eq!(row.concat(), "qwertyuıopğü");
        let row: Vec<&str> = (code::AC01..=code::AC11).map(|c| t.label(c, PLAIN).unwrap()).collect();
        assert_eq!(row.concat(), "asdfghjklşi");
        let row: Vec<&str> = (code::AB01..=code::AB10).map(|c| t.label(c, PLAIN).unwrap()).collect();
        assert_eq!(row.concat(), "zxcvbnmöç.");
        assert_eq!(t.label(code::BKSL, PLAIN), Some(","));
        assert_eq!(t.label(code::TLDE, PLAIN), Some("\""));
        // Türkçe i/ı büyük harfleri.
        assert_eq!(t.label(code::AC11, SHIFT), Some("İ"));
        assert_eq!(t.label(code::AD08, SHIFT), Some("I"));
        assert_eq!(t.label(code::AC11, CAPS), Some("İ"));
        assert_eq!(t.label(code::AD08, CAPS), Some("I"));
        assert_eq!(t.label(code::AC10, CAPS), Some("Ş"));
        // Caps + Shift harfi küçültür; rakam satırı Caps'ten etkilenmez.
        assert_eq!(t.label(code::AC11, Level { shift: true, caps: true, altgr: false }), Some("i"));
        assert_eq!(t.label(code::AE01, CAPS), Some("1"));
        let row: Vec<&str> = (code::AE01..=code::AE12).map(|c| t.label(c, SHIFT).unwrap()).collect();
        assert_eq!(row.concat(), "!'^+%&/()=?_");
    }

    #[test]
    fn turkish_q_altgr_matches_real_xkb() {
        let t = tr();
        let at = |c| t.label(c, ALTGR);
        assert_eq!(at(code::AD01), Some("@"));
        assert_eq!(at(code::AD01 + 2), Some("€"), "e");
        assert_eq!(at(code::AD01 + 4), Some("₺"), "t");
        assert_eq!(at(code::AE01 + 6), Some("{"), "7");
        assert_eq!(at(code::AE01 + 7), Some("["), "8");
        assert_eq!(at(code::AE01 + 8), Some("]"), "9");
        assert_eq!(at(code::AE01 + 9), Some("}"), "0");
        assert_eq!(at(code::AE01 + 10), Some("\\"), "*");
        assert_eq!(at(code::AE12), Some("|"), "-");
        assert_eq!(at(code::AD12), Some("~"), "ü");
        assert_eq!(at(code::AE01 + 2), Some("#"), "3");
        assert_eq!(at(code::AE01 + 3), Some("$"), "4");
        assert_eq!(at(code::LSGT), Some("|"));
        assert_eq!(at(code::TLDE), Some("<"));
        assert_eq!(at(code::AE01), Some(">"));
        assert_eq!(at(code::BKSL), Some("`"));
        // 3. seviyesi olmayan tuş: sembol yok (etkisiz).
        assert_eq!(at(code::AD01 + 1), None, "w");
        // Ölü tuşlar boşluklu işaretle.
        assert_eq!(t.label(code::AD12, SHIFT_ALTGR), Some("¯"));
        assert!(t.has_level3());
    }

    #[test]
    fn us_layout() {
        let u = Labels::compile(Layout::Us).unwrap();
        let row: Vec<&str> = (code::AD01..=code::AD12).map(|c| u.label(c, PLAIN).unwrap()).collect();
        assert_eq!(row.concat(), "qwertyuiop[]");
        let row: Vec<&str> = (code::AE01..=code::AE12).map(|c| u.label(c, SHIFT).unwrap()).collect();
        assert_eq!(row.concat(), "!@#$%^&*()_+");
        assert_eq!(u.label(code::AC11, PLAIN), Some("'"));
        assert_eq!(u.label(code::AC11, CAPS), Some("'"));
        assert_eq!(u.label(code::AD01 + 7, CAPS), Some("I"), "US'te i → I");
        assert_eq!(u.label(code::BKSL, PLAIN), Some("\\"));
        assert_eq!(u.label(code::TLDE, SHIFT), Some("~"));
        // pc105 US: AltGr yalnızca <> tuşunda (| ¦); diğer tuşlar AltGr ile aynı harfi verir.
        assert!(u.has_level3());
        assert_eq!(u.label(code::LSGT, Level { shift: false, caps: false, altgr: true }), Some("|"));
        assert_eq!(u.label(code::AD01, Level { shift: false, caps: false, altgr: true }), Some("q"));
    }

    #[test]
    fn layout_names() {
        assert_eq!(Layout::from_name("tr"), Some(Layout::Tr));
        assert_eq!(Layout::from_name("us"), Some(Layout::Us));
        assert_eq!(Layout::from_name("TR"), None);
        assert_eq!(Layout::from_xkb_name("Turkish"), Layout::Tr);
        assert_eq!(Layout::from_xkb_name("Turkish (F)"), Layout::Us, "F düzeni Q değil");
        assert_eq!(Layout::from_xkb_name("English (US)"), Layout::Us);
        assert_eq!(Layout::Tr.next(), Layout::Us);
        assert_eq!(Layout::Us.next(), Layout::Tr);
    }
}

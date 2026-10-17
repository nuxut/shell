//! Ekran klavyesinin fiziksel tuş tablosu (ISO 105 tuşlu klavyenin yazı bölümü, ANSI'ye yakın
//! dizilim). Her iki düzen (TR-Q, US) aynı fiziksel tuşları kullanır; etiketler düzenin keymap'inden
//! türetilir ([`super::keymap`]), burada yalnızca evdev kodu, genişlik ve tuşun türü vardır.
//!
//! Satırlar 15 birim genişliğindedir (1 birim = standart tuş); görünüm satırı yüzey genişliğine
//! ölçekler.

/// evdev tuş kodları (`linux/input-event-codes.h`). Sanal klavye protokolü bu kodları alır;
/// xkb keycode'u bunun 8 fazlasıdır.
pub mod code {
    pub const AE01: u32 = 2;
    pub const AE12: u32 = 13;
    pub const BACKSPACE: u32 = 14;
    pub const TAB: u32 = 15;
    pub const AD01: u32 = 16;
    pub const AD08: u32 = 23;
    pub const AD12: u32 = 27;
    pub const ENTER: u32 = 28;
    pub const AC01: u32 = 30;
    pub const AC10: u32 = 39;
    pub const AC11: u32 = 40;
    pub const TLDE: u32 = 41;
    pub const BKSL: u32 = 43;
    pub const AB01: u32 = 44;
    pub const AB10: u32 = 53;
    pub const SPACE: u32 = 57;
    pub const LSGT: u32 = 86;
    pub const UP: u32 = 103;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const DOWN: u32 = 108;
}

/// Yazı üretmeyen, sabit etiketli tuşlar (keymap'ten bağımsız).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Special {
    Backspace,
    Tab,
    Enter,
    Space,
    Left,
    Right,
    Up,
    Down,
}

impl Special {
    pub const fn code(self) -> u32 {
        match self {
            Special::Backspace => code::BACKSPACE,
            Special::Tab => code::TAB,
            Special::Enter => code::ENTER,
            Special::Space => code::SPACE,
            Special::Left => code::LEFT,
            Special::Right => code::RIGHT,
            Special::Up => code::UP,
            Special::Down => code::DOWN,
        }
    }

    /// Basılı tutunca tekrarlar mı (silme ve ok tuşları).
    pub const fn repeats(self) -> bool {
        matches!(self, Special::Backspace | Special::Left | Special::Right | Special::Up | Special::Down)
    }
}

/// Bir tuşun yaptığı iş.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyAction {
    /// Düzenin keymap'ine göre karakter üreten tuş (evdev kodu).
    Char(u32),
    Special(Special),
    /// Tek vuruşluk Shift (bir sonraki tuşa uygulanır).
    Shift,
    /// Büyük harf kilidi.
    Caps,
    /// Tek vuruşluk AltGr (ISO_Level3_Shift; yalnızca düzende 3. seviye varsa görünür).
    AltGr,
    /// Tek vuruşluk Ctrl.
    Ctrl,
    /// Sıradaki düzene geç (TR ↔ US).
    SwitchLayout,
    /// Klavyeyi gizle.
    Hide,
}

/// Tablodaki bir tuş.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeySpec {
    pub action: KeyAction,
    /// Birim genişlik.
    pub width: f32,
}

const fn k(action: KeyAction, width: f32) -> KeySpec {
    KeySpec { action, width }
}

const fn c(code: u32) -> KeySpec {
    k(KeyAction::Char(code), 1.0)
}

/// Satır genişliği (birim).
pub const ROW_UNITS: f32 = 15.0;

/// Satırlar (üstten alta).
pub fn rows() -> [Vec<KeySpec>; 5] {
    use KeyAction as A;
    use Special as S;
    let range = |from: u32, to: u32| (from..=to).map(c);
    let mut r0 = vec![c(code::TLDE)];
    r0.extend(range(code::AE01, code::AE12));
    r0.push(k(A::Special(S::Backspace), 2.0));

    let mut r1 = vec![k(A::Special(S::Tab), 1.5)];
    r1.extend(range(code::AD01, code::AD12));
    r1.push(k(A::Char(code::BKSL), 1.5));

    let mut r2 = vec![k(A::Caps, 1.75)];
    r2.extend(range(code::AC01, code::AC11));
    r2.push(k(A::Special(S::Enter), 2.25));

    let mut r3 = vec![k(A::Shift, 1.25), c(code::LSGT)];
    r3.extend(range(code::AB01, code::AB10));
    r3.push(k(A::Special(S::Up), 1.0));
    r3.push(k(A::Shift, 1.75));

    let r4 = vec![
        k(A::Ctrl, 1.5),
        k(A::SwitchLayout, 1.5),
        k(A::Special(S::Space), 6.0),
        k(A::AltGr, 1.5),
        k(A::Special(S::Left), 1.0),
        k(A::Special(S::Down), 1.0),
        k(A::Special(S::Right), 1.0),
        k(A::Hide, 1.5),
    ];
    [r0, r1, r2, r3, r4]
}

/// Tablodaki bütün karakter tuşlarının evdev kodları (sırayla).
pub fn char_codes() -> Vec<u32> {
    rows()
        .iter()
        .flatten()
        .filter_map(|s| match s.action {
            KeyAction::Char(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fill_the_width() {
        for (i, row) in rows().iter().enumerate() {
            let w: f32 = row.iter().map(|k| k.width).sum();
            assert!((w - ROW_UNITS).abs() < 1e-4, "satır {i}: {w}");
        }
    }

    #[test]
    fn every_char_key_is_unique_and_in_the_typing_block() {
        let codes = char_codes();
        let mut sorted = codes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "tekrarlanan tuş yok");
        // 13 + 12 + 1 + 11 + 1 + 10 = 48 yazı tuşu (ISO).
        assert_eq!(codes.len(), 48);
        for c in codes {
            assert!(matches!(c, code::AE01..=code::AE12 | code::AD01..=code::AD12 | code::AC01..=code::AC11 | code::TLDE | code::BKSL | code::AB01..=code::AB10 | code::LSGT), "{c}");
        }
    }

    #[test]
    fn specials_have_fixed_codes() {
        assert_eq!(Special::Backspace.code(), 14);
        assert_eq!(Special::Enter.code(), 28);
        assert_eq!(Special::Space.code(), 57);
        assert!(Special::Backspace.repeats() && Special::Left.repeats());
        assert!(!Special::Enter.repeats() && !Special::Space.repeats() && !Special::Tab.repeats());
    }
}

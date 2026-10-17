//! `zvariant::Value` okuma yardımcıları. Uygulamadan gelen her değer güvenilmezdir: yanlış
//! tip `None` (varsayılan) olur, metinler sınırlanır, hiçbir yol panik etmez.

use zbus::zvariant::Value;

/// İç içe variant katmanlarını soyar (`v` içinde `v`). D-Bus imza derinliği zaten
/// sınırlıdır (≤ 64); döngü yine de sabit bir üst sınırla kesilir.
pub(crate) fn peel<'a, 'v>(mut value: &'a Value<'v>) -> &'a Value<'v> {
    for _ in 0..64 {
        match value {
            Value::Value(inner) => value = inner,
            _ => break,
        }
    }
    value
}

pub(crate) fn as_str<'a>(value: &'a Value<'_>) -> Option<&'a str> {
    match peel(value) {
        Value::Str(s) => Some(s.as_str()),
        Value::ObjectPath(p) => Some(p.as_str()),
        _ => None,
    }
}

pub(crate) fn as_bool(value: &Value<'_>) -> Option<bool> {
    match peel(value) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// Tamsayı; bazı uygulamalar `i` yerine `u`/`x` gönderir, aralığa sığan her tip kabul edilir.
pub(crate) fn as_i64(value: &Value<'_>) -> Option<i64> {
    match peel(value) {
        Value::U8(v) => Some(i64::from(*v)),
        Value::I16(v) => Some(i64::from(*v)),
        Value::U16(v) => Some(i64::from(*v)),
        Value::I32(v) => Some(i64::from(*v)),
        Value::U32(v) => Some(i64::from(*v)),
        Value::I64(v) => Some(*v),
        Value::U64(v) => i64::try_from(*v).ok(),
        _ => None,
    }
}

pub(crate) fn as_array<'a, 'v>(value: &'a Value<'v>) -> Option<&'a [Value<'v>]> {
    match peel(value) {
        Value::Array(a) => Some(a.inner()),
        _ => None,
    }
}

pub(crate) fn as_fields<'a, 'v>(value: &'a Value<'v>) -> Option<&'a [Value<'v>]> {
    match peel(value) {
        Value::Structure(s) => Some(s.fields()),
        _ => None,
    }
}

/// `ay` dizisini bayt dilimine çevirir; `limit` baytı aşarsa `None`.
pub(crate) fn as_bytes(value: &Value<'_>, limit: usize) -> Option<Vec<u8>> {
    let items = as_array(value)?;
    if items.len() > limit {
        return None;
    }
    items
        .iter()
        .map(|v| match v {
            Value::U8(b) => Some(*b),
            _ => None,
        })
        .collect()
}

/// Metni en fazla `max` karaktere kırpar ve denetim karakterlerini (satır sonu hariç)
/// atar: uygulama başlığı çizim tarafını bozamaz.
pub(crate) fn clip(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').take(max).collect()
}

/// Tek satırlık metin (başlık, kimlik, ikon adı): satır sonları da atılır.
pub(crate) fn clip_line(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

#[cfg(test)]
mod tests {
    use zbus::zvariant::{Array, Structure, Value};

    use super::*;

    #[test]
    fn peel_unwraps_nested_variants() {
        let v = Value::Value(Box::new(Value::Value(Box::new(Value::I32(7)))));
        assert_eq!(as_i64(&v), Some(7));
        assert_eq!(as_str(&v), None);
    }

    #[test]
    fn integer_widths_are_accepted() {
        assert_eq!(as_i64(&Value::U32(5)), Some(5));
        assert_eq!(as_i64(&Value::U64(u64::MAX)), None);
        assert_eq!(as_i64(&Value::from("x")), None);
    }

    #[test]
    fn bytes_respect_limit_and_type() {
        let arr = Value::Array(Array::from(vec![1u8, 2, 3]));
        assert_eq!(as_bytes(&arr, 3), Some(vec![1, 2, 3]));
        assert_eq!(as_bytes(&arr, 2), None);
        let wrong = Value::Array(Array::from(vec![1i32]));
        assert_eq!(as_bytes(&wrong, 8), None);
        let s = Value::Structure(Structure::from((1i32, 2i32)));
        assert_eq!(as_fields(&s).map(<[_]>::len), Some(2));
    }

    #[test]
    fn clip_removes_control_and_limits() {
        assert_eq!(clip_line("a\u{1b}[31mb\nc", 10), "a[31mbc");
        assert_eq!(clip("ğğğ\nx", 4), "ğğğ\n");
    }
}

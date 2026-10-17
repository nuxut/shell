//! Ayar türünün şeması, `Deserialize` uygulamasından çıkarılır: hangi anahtarlar var, türleri ne.
//!
//! Sahte bir `Deserializer` türü "her alanı var" gibi sürer ve serde'nin `deserialize_struct`'a
//! verdiği alan adlarını kaydeder. Her alan ayrı bir çalıştırmada izlenir (yalnızca o alan dolu):
//! bir alanın özel `Deserialize`'ı hata verse de diğerleri etkilenmez; kayıt yan etkidir, sonuç
//! değeri kullanılmaz. Haritalar (`BTreeMap<String, T>`) `*` parçasıyla, değer türü bir kez izlenir.
//!
//! Kullanım: yazım hatası önerisinin adayları ve şema anlık görüntüsü (göç bekçisi testi).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess, Visitor};

/// Bir anahtarın türü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int,
    Float,
    Str,
    Seq,
    /// Serbest anahtarlı tablo; değerin şeması `<yol>.*` altında.
    Map,
    /// Alanları bilinen tablo.
    Struct(Vec<&'static str>),
    /// Birim varyantlı enum (dizede yazılır) ya da genel enum.
    Enum(Vec<&'static str>),
    /// Türü izlenemeyen değer (`toml::Value`, `deserialize_any`).
    Any,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => f.write_str("bool"),
            Self::Int => f.write_str("int"),
            Self::Float => f.write_str("float"),
            Self::Str => f.write_str("str"),
            Self::Seq => f.write_str("seq"),
            Self::Map => f.write_str("map"),
            Self::Struct(_) => f.write_str("table"),
            Self::Enum(v) => write!(f, "enum({})", v.join("|")),
            Self::Any => f.write_str("any"),
        }
    }
}

/// Yol (`.` ile birleştirilmiş, harita anahtarı `*`) → tür.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    nodes: BTreeMap<String, Kind>,
    root: Vec<&'static str>,
}

/// Sonsuz özyinelemeye karşı derinlik sınırı.
const MAX_DEPTH: usize = 16;

impl Schema {
    /// `T`'nin şeması.
    pub fn of<'de, T: de::Deserialize<'de>>() -> Self {
        let rec = RefCell::new(BTreeMap::new());
        let mut queue: Vec<Vec<String>> = vec![Vec::new()];
        while let Some(focus) = queue.pop() {
            let _ = T::deserialize(Tracer { path: Vec::new(), focus: &focus, rec: &rec });
            if focus.len() >= MAX_DEPTH {
                continue;
            }
            let kind = rec.borrow().get(&focus.join(".")).cloned();
            match kind {
                Some(Kind::Struct(fields)) => {
                    for f in fields {
                        let mut p = focus.clone();
                        p.push(f.to_owned());
                        if !rec.borrow().contains_key(&p.join(".")) {
                            queue.push(p);
                        }
                    }
                }
                Some(Kind::Map) => {
                    let mut p = focus.clone();
                    p.push("*".to_owned());
                    if !rec.borrow().contains_key(&p.join(".")) {
                        queue.push(p);
                    }
                }
                _ => {}
            }
        }
        let mut nodes = rec.into_inner();
        let root = match nodes.remove("") {
            Some(Kind::Struct(f)) => f,
            _ => Vec::new(),
        };
        Self { nodes, root }
    }

    pub fn get(&self, path: &str) -> Option<&Kind> {
        self.nodes.get(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Kind)> {
        self.nodes.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Dosyadaki yolun şemadaki karşılığı (harita anahtarları `*`). Yol şemada yoksa `None`.
    pub fn resolve(&self, path: &[String]) -> Option<String> {
        let mut out: Vec<&str> = Vec::new();
        for seg in path {
            let parent = out.join(".");
            let parent_kind = if out.is_empty() { None } else { self.nodes.get(&parent) };
            match parent_kind {
                Some(Kind::Map) => out.push("*"),
                _ => out.push(seg),
            }
            if !self.nodes.contains_key(&out.join(".")) {
                return None;
            }
        }
        Some(out.join("."))
    }

    /// `parent`'ın (şema yolu; kök için boş) bilinen alt anahtarları.
    pub fn children(&self, parent: &str) -> Vec<&'static str> {
        if parent.is_empty() {
            return self.root.clone();
        }
        match self.nodes.get(parent) {
            Some(Kind::Struct(f)) => f.clone(),
            _ => Vec::new(),
        }
    }

    /// Anlık görüntü metni: her satır `yol tür`, sıralı.
    pub fn snapshot(&self) -> String {
        self.nodes.iter().map(|(k, v)| format!("{k} {v}\n")).collect()
    }
}

struct Tracer<'a> {
    path: Vec<String>,
    focus: &'a [String],
    rec: &'a RefCell<BTreeMap<String, Kind>>,
}

#[derive(Debug)]
struct TraceError;

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("trace")
    }
}

impl std::error::Error for TraceError {}

impl de::Error for TraceError {
    fn custom<T: fmt::Display>(_: T) -> Self {
        TraceError
    }
}

impl<'a> Tracer<'a> {
    fn record(&self, kind: Kind) {
        let key = self.path.join(".");
        let mut rec = self.rec.borrow_mut();
        // `Option<T>` aynı yolu iki kez görebilir; bilinen tür `Any`'yi ezer.
        match rec.get(&key) {
            Some(k) if *k != Kind::Any => {}
            _ => {
                rec.insert(key, kind);
            }
        }
    }

    fn child(&self, seg: &str) -> Tracer<'a> {
        let mut path = self.path.clone();
        path.push(seg.to_owned());
        Tracer { path, focus: self.focus.get(1..).unwrap_or(&[]), rec: self.rec }
    }
}

/// Tek girdili (ya da boş) harita: odaktaki alan.
struct One<'a> {
    tracer: Option<Tracer<'a>>,
    key: Option<String>,
}

impl<'de, 'a> MapAccess<'de> for One<'a> {
    type Error = TraceError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, TraceError> {
        match self.key.take() {
            Some(k) => seed.deserialize(k.into_deserializer()).map(Some),
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, TraceError> {
        match self.tracer.take() {
            Some(t) => seed.deserialize(t),
            None => Err(TraceError),
        }
    }
}

struct Empty;

impl<'de> SeqAccess<'de> for Empty {
    type Error = TraceError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, _: T) -> Result<Option<T::Value>, TraceError> {
        Ok(None)
    }
}

/// İlk varyantı seçen enum erişimi.
struct FirstVariant(&'static str);

impl<'de> EnumAccess<'de> for FirstVariant {
    type Error = TraceError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), TraceError> {
        let v = seed.deserialize(self.0.into_deserializer())?;
        Ok((v, self))
    }
}

impl<'de> VariantAccess<'de> for FirstVariant {
    type Error = TraceError;

    fn unit_variant(self) -> Result<(), TraceError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, _: T) -> Result<T::Value, TraceError> {
        Err(TraceError)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, _: V) -> Result<V::Value, TraceError> {
        Err(TraceError)
    }

    fn struct_variant<V: Visitor<'de>>(self, _: &'static [&'static str], _: V) -> Result<V::Value, TraceError> {
        Err(TraceError)
    }
}

impl<'de, 'a> Deserializer<'de> for Tracer<'a> {
    type Error = TraceError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Any);
        visitor.visit_unit()
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Bool);
        visitor.visit_bool(false)
    }

    fn deserialize_i8<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_i64(v)
    }
    fn deserialize_i16<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_i64(v)
    }
    fn deserialize_i32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_i64(v)
    }
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Int);
        visitor.visit_i64(0)
    }
    fn deserialize_u8<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_u64(v)
    }
    fn deserialize_u16<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_u64(v)
    }
    fn deserialize_u32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_u64(v)
    }
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Int);
        visitor.visit_u64(0)
    }
    fn deserialize_f32<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_f64(v)
    }
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Float);
        visitor.visit_f64(0.0)
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Str);
        visitor.visit_char(' ')
    }
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Str);
        visitor.visit_str("")
    }
    fn deserialize_string<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_str(v)
    }
    fn deserialize_bytes<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_any(v)
    }
    fn deserialize_byte_buf<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_any(v)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Any);
        visitor.visit_unit()
    }
    fn deserialize_unit_struct<V: Visitor<'de>>(self, _: &'static str, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_unit(v)
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _: &'static str, visitor: V) -> Result<V::Value, TraceError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Seq);
        visitor.visit_seq(Empty)
    }
    fn deserialize_tuple<V: Visitor<'de>>(self, _: usize, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_seq(v)
    }
    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _: &'static str, _: usize, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_seq(v)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        self.record(Kind::Map);
        match self.focus.first() {
            Some(seg) => {
                let key = seg.clone();
                let tracer = self.child(seg);
                visitor.visit_map(One { tracer: Some(tracer), key: Some(key) })
            }
            None => visitor.visit_map(One { tracer: None, key: None }),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, TraceError> {
        self.record(Kind::Struct(fields.to_vec()));
        match self.focus.first() {
            Some(seg) => {
                let key = seg.clone();
                let tracer = self.child(seg);
                visitor.visit_map(One { tracer: Some(tracer), key: Some(key) })
            }
            None => visitor.visit_map(One { tracer: None, key: None }),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, TraceError> {
        self.record(Kind::Enum(variants.to_vec()));
        match variants.first() {
            Some(v) => visitor.visit_enum(FirstVariant(v)),
            None => Err(TraceError),
        }
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, v: V) -> Result<V::Value, TraceError> {
        self.deserialize_str(v)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, TraceError> {
        visitor.visit_unit()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize, Default)]
    #[serde(default)]
    #[allow(dead_code)]
    struct Root {
        bar: Bar,
        outputs: BTreeMap<String, Out>,
        version: Option<toml::Value>,
        list: Vec<String>,
        #[serde(rename = "kind")]
        k: Edge,
        strict: Strict,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    #[allow(dead_code)]
    struct Bar {
        size: f32,
        edge: Option<Edge>,
        enabled: bool,
        count: u32,
    }

    #[derive(Deserialize, Default)]
    #[serde(rename_all = "lowercase")]
    enum Edge {
        #[default]
        Top,
        Bottom,
    }

    #[derive(Deserialize, Default)]
    #[allow(dead_code)]
    struct Out {
        path: String,
    }

    /// `#[serde(default)]`'suz yapı ve hata veren alan diğerlerini engellemez.
    #[derive(Deserialize, Default)]
    #[allow(dead_code)]
    struct Strict {
        a: Edge,
        b: i64,
    }

    #[test]
    fn traces_fields_kinds_and_maps() {
        let s = Schema::of::<Root>();
        assert_eq!(
            s.snapshot(),
            "\
bar table
bar.count int
bar.edge enum(top|bottom)
bar.enabled bool
bar.size float
kind enum(top|bottom)
list seq
outputs map
outputs.* table
outputs.*.path str
strict table
strict.a enum(top|bottom)
strict.b int
version any
"
        );
        assert_eq!(s.resolve(&["outputs".into(), "DP-1".into(), "path".into()]), Some("outputs.*.path".into()));
        assert_eq!(s.resolve(&["bar".into(), "siez".into()]), None);
        assert_eq!(s.children("bar"), ["size", "edge", "enabled", "count"]);
        assert_eq!(s.children("").len(), 6);
    }
}

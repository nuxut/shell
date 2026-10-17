//! Gizli bilgi (Wi-Fi şifresi, WireGuard anahtarı) taşıyıcısı ve NM'ye kopyasız yazımı.
//!
//! **Kopya politikası.** Şifre [`Secret`] içinde tek bir sıfırlanan (`Zeroizing`) tamponda
//! durur. Girişte: [`Secret::from_bytes`] çağıranın `Zeroizing<Vec<u8>>` tamponunu kopyalamadan
//! sahiplenir (Ayarlar: `SecretBuffer::to_zeroizing()` → tek kopya); [`Secret::from_utf8`]
//! ödünç bayttan tek (sıfırlanan) kopya yapar. NM'ye giden ayarlarda değer `Value::Str`
//! olarak şifreden **ödünç** alınır ve doğrudan mesaja serileştirilir (ara `String` /
//! `OwnedValue` yok). **Bilinen sınır:** zbus'ın mesaj tamponu (gönderilen bayt dizisi)
//! sıfırlanmaz; NM de şifreyi kendi belleğinde/diskte tutar.

use std::collections::HashMap;

use zbus::export::serde::ser::{Serialize, SerializeMap, Serializer};
use zbus::zvariant::{OwnedValue, Signature, Type, Value};
use zeroize::{Zeroize, Zeroizing};

/// Bir şifre ya da anahtar. `Debug` içeriği göstermez; düşürülünce bellek sıfırlanır.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    /// `String`'i sahiplenir (kopyasız). `&str` verilirse bir kopya yapılır; o kopya da
    /// sıfırlanır ama kaynağı çağıran sıfırlamalıdır.
    pub fn new(s: impl Into<String>) -> Self {
        Self(Zeroizing::new(s.into()))
    }

    /// Sıfırlanan bayt tamponunu kopyalamadan sahiplenir (UTF-8 doğrulamalı). Geçersiz
    /// UTF-8'de tampon sıfırlanır ve `None` döner.
    pub fn from_bytes(mut bytes: Zeroizing<Vec<u8>>) -> Option<Self> {
        let raw = std::mem::take(&mut *bytes);
        match String::from_utf8(raw) {
            Ok(s) => Some(Self(Zeroizing::new(s))),
            Err(e) => {
                let mut raw = e.into_bytes();
                raw.zeroize();
                None
            }
        }
    }

    /// Ödünç bayttan tek (sıfırlanan) kopya; geçersiz UTF-8'de `None`.
    pub fn from_utf8(bytes: &[u8]) -> Option<Self> {
        std::str::from_utf8(bytes).ok().map(|s| Self(Zeroizing::new(s.to_owned())))
    }

    /// Bayt uzunluğu.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    /// NM ayarı için ödünç değer (kopya yok).
    pub(crate) fn value(&self) -> Value<'_> {
        Value::from(self.expose())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<gizli>)")
    }
}

/// `SecretAgent.GetSecrets` cevabı (`a{sa{sv}}`): tek ayar grubu, tek anahtar. Değer
/// serileştirilirken şifreden ödünç alınır; cevap düşünce şifre sıfırlanır.
pub(crate) struct SecretsReply {
    pub setting: String,
    pub key: &'static str,
    pub secret: Secret,
}

impl Type for SecretsReply {
    const SIGNATURE: &'static Signature = <HashMap<String, HashMap<String, OwnedValue>> as Type>::SIGNATURE;
}

impl Serialize for SecretsReply {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Inner<'a>(&'a SecretsReply);
        impl Serialize for Inner<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut m = serializer.serialize_map(Some(1))?;
                m.serialize_entry(self.0.key, &self.0.secret.value())?;
                m.end()
            }
        }
        let mut m = serializer.serialize_map(Some(1))?;
        m.serialize_entry(&self.setting, &Inner(self))?;
        m.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::serialized::Context;
    use zbus::zvariant::{LE, to_bytes};

    #[test]
    fn redacted_and_validated() {
        let s = Secret::new("çokgizli123");
        assert_eq!(format!("{s:?}"), "Secret(<gizli>)");
        let ok = Secret::from_bytes(Zeroizing::new(b"parola".to_vec())).unwrap();
        assert_eq!(ok.expose(), "parola");
        assert_eq!(ok.len(), 6);
        assert!(Secret::from_bytes(Zeroizing::new(vec![0xff, 0xfe])).is_none());
        assert_eq!(Secret::from_utf8("ş".as_bytes()).unwrap().expose(), "ş");
        assert!(Secret::from_utf8(&[0xc3]).is_none());
    }

    #[test]
    fn secrets_reply_wire_format() {
        let reply = SecretsReply {
            setting: "802-11-wireless-security".into(),
            key: "psk",
            secret: Secret::new("parola123"),
        };
        let ctx = Context::new_dbus(LE, 0);
        let ours = to_bytes(ctx, &reply).unwrap();
        let reference: HashMap<String, HashMap<String, OwnedValue>> = HashMap::from([(
            "802-11-wireless-security".to_owned(),
            HashMap::from([("psk".to_owned(), OwnedValue::try_from(Value::from("parola123")).unwrap())]),
        )]);
        let theirs = to_bytes(ctx, &reference).unwrap();
        assert_eq!(ours.bytes(), theirs.bytes());
    }
}

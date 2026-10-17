//! Yardımcının çıktısı: stdout'a satır başına bir JSON olay. Metin yok (dil bağımsız); kullanıcı
//! tarafı (`nuxut-shell system-theme`) olayları çevirip gösterir.

use std::io::Write;
use std::sync::{Arc, Mutex};

use nuxut_boot_theme::plan::{Plan, Target};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "event")]
pub enum Event {
    /// Yardımcının kendi algılamasıyla kurduğu plan (işe başlamadan önce).
    Plan { plan: Plan },
    /// Bir hedefin bir adımı.
    Step { target: Target, step: String, state: StepState },
    /// Dış komut çıktısı (satır satır, denetim karakterleri ayıklanmış).
    Log { source: String, line: String },
    /// Uyarı (iş sürüyor).
    Warn { target: Option<Target>, message: String },
    /// Hedef sonucu.
    Target { target: Target, outcome: Outcome, message: Option<String> },
    /// Son satır.
    Done { ok: bool, reboot_hint: bool },
    /// Giriş ekranı alanının durumu (`greeter status`).
    LoginScreen { status: crate::greeter::Status },
    /// Giriş ekranı alanının adımı (`record`, `config`, `theme`, `display-manager`, `keymap`).
    LoginStep { step: String, state: StepState },
    /// Giriş ekranı alanının sonucu; `message` dil bağımsız kod (`greetd-missing` …) ya da hata.
    LoginResult { outcome: Outcome, message: Option<String> },
    /// Paket alanının sonucu (`packages install`): `applied` kuruldu, `rolled-back` pacman başarısız
    /// (`message` tanı metni).
    PackagesResult { outcome: Outcome, names: Vec<String>, message: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    Start,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Applied,
    Reverted,
    /// Uygulanamadı ve bu turdaki değişiklikler geri alındı.
    RolledBack,
    /// Atlandı (kurulu değil, engelli ya da yapılacak iş yok).
    Skipped,
    /// Geri alma da başarısız: sistem karışık durumda (mesajda yedek yolları).
    Inconsistent,
}

/// Olay yazıcısı (thread'ler arası paylaşılır; komut çıktısı okuyucuları da yazar).
#[derive(Debug, Clone, Default)]
pub struct Events {
    sink: Option<Arc<Mutex<Vec<Event>>>>,
}

impl Events {
    /// stdout'a yazan.
    pub fn stdout() -> Self {
        Self { sink: None }
    }

    /// Bellekte biriktiren (testler).
    pub fn memory() -> (Self, Arc<Mutex<Vec<Event>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        (Self { sink: Some(buf.clone()) }, buf)
    }

    pub fn emit(&self, e: Event) {
        match &self.sink {
            Some(buf) => {
                if let Ok(mut b) = buf.lock() {
                    b.push(e);
                }
            }
            None => {
                if let Ok(line) = serde_json::to_string(&e) {
                    let mut out = std::io::stdout().lock();
                    let _ = writeln!(out, "{line}");
                    let _ = out.flush();
                }
            }
        }
    }

    pub fn step(&self, target: Target, step: &str, state: StepState) {
        self.emit(Event::Step { target, step: step.to_owned(), state });
    }

    pub fn warn(&self, target: Option<Target>, message: impl Into<String>) {
        self.emit(Event::Warn { target, message: message.into() });
    }
}

/// Komut çıktısı satırını güvenli hâle getirir: geçersiz UTF-8 kayıpla, denetim karakterleri
/// (sekme hariç) atılır, en çok 1000 karakter.
pub fn sanitize(line: &[u8]) -> String {
    String::from_utf8_lossy(line).chars().filter(|c| !c.is_control() || *c == '\t').take(1000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_escape_sequences() {
        assert_eq!(sanitize(b"\x1b[31mred\x1b[0m\r"), "[31mred[0m");
        assert_eq!(sanitize(&[0xff, b'a']), "\u{fffd}a");
        assert_eq!(sanitize(&[b'x'; 5000]).len(), 1000);
    }

    #[test]
    fn events_serialize_tagged() {
        let e = Event::Target { target: Target::Grub, outcome: Outcome::RolledBack, message: Some("x".into()) };
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(s, r#"{"event":"target","target":"grub","outcome":"rolled-back","message":"x"}"#);
        assert_eq!(serde_json::from_str::<Event>(&s).unwrap(), e);
    }
}

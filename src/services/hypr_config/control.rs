//! Üretilen dosyanın canlı uygulanması: ne gönderileceği saf olarak planlanır
//! ([`plan_apply`]), gönderim [`HyprControl`] arkasındadır (testlerde sahte).

use std::collections::BTreeSet;

use super::generate::{Generated, safe_value};
use super::input::RELOAD_KEYS;

/// Hyprland'e yazma erişimi. Gerçek uygulama `HyprlandClient` soketi; testlerde kayıtçı.
pub trait HyprControl {
    /// Ham istek (`[[BATCH]]…`, `reload config-only`, `configerrors`); cevap metni.
    fn request(&self, command: &str) -> Result<String, String>;
}

/// Gerçek uygulama: Hyprland `socket1`. Yalnızca [`super::sync::SyncEnv::may_control`]
/// doğruyken (kullanıcının oturumu, test örneği değil) kurulur.
impl HyprControl for crate::services::hyprland::HyprlandClient {
    fn request(&self, command: &str) -> Result<String, String> {
        crate::services::hyprland::HyprlandClient::request(self, command).map_err(|e| e.to_string())
    }
}

/// `eval` kodu üst sınırı (Hyprland 250 ms zaman aşımıyla çalıştırır).
pub const MAX_EVAL_BYTES: usize = 64 * 1024;

/// Uygulama adımı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyAction {
    /// Dosya değişmedi.
    Nothing,
    /// Yalnızca değerler değişti: tek batch.
    Keywords(String),
    /// Bind ya da kaldırılan anahtar var: Hyprland ayarı yeniden okumalı.
    Reload,
    /// Lua: tek satırlık kod `hyprctl eval` ile (yalnızca değerler; ADR-0003 Karar 3).
    Eval(String),
    /// Ayar yolunu yeniden seçerek tam yeniden yükleme (`.conf`→`.lua` geçişi; yalnızca kullanıcı
    /// "Şimdi uygula" dediğinde — ADR-0003 Karar 4).
    FullReset,
}

/// `[[BATCH]]keyword k v;keyword …`. Anahtar ya da değer güvensizse `None`.
pub fn keyword_batch(keywords: &[(String, String)]) -> Option<String> {
    let key_ok = |k: &str| !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b"_:.-".contains(&b));
    let mut out = String::from("[[BATCH]]");
    for (i, (k, v)) in keywords.iter().enumerate() {
        if !key_ok(k) || !safe_value(v) {
            return None;
        }
        if i > 0 {
            out.push(';');
        }
        out.push_str("keyword ");
        out.push_str(k);
        out.push(' ');
        out.push_str(v);
    }
    Some(out)
}

/// Metindeki (anahtar, değer) satırları; yorumlar hariç.
fn lines(text: &str) -> Vec<(&str, &str)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
        .collect()
}

fn is_bind(key: &str) -> bool {
    key.starts_with("bind")
}

/// Değeri yalnızca yeniden okumayla uygulanan anahtar mı ([`RELOAD_KEYS`]: klavye düzeni)?
fn is_reload_key(key: &str) -> bool {
    RELOAD_KEYS.contains(&key)
}

/// Önceki dosya metni (yoksa `None`) ile yeni çıktıyı karşılaştırır.
pub fn plan_apply(old: Option<&str>, new: &Generated) -> ApplyAction {
    let Some(old) = old else {
        // İlk üretim: dosya yeni source edildiyse Hyprland zaten okur; değilse değerler yine de
        // uygulanır, bind'ler ve klavye düzeni için yeniden okuma gerekir.
        let structural = lines(&new.text).iter().any(|(k, _)| is_bind(k) || is_reload_key(k));
        return if structural { ApplyAction::Reload } else { batch(new) };
    };
    if old == new.text {
        return ApplyAction::Nothing;
    }
    let (old_lines, new_lines) = (lines(old), lines(&new.text));
    fn binds<'a>(ls: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        ls.iter().filter(|(k, _)| is_bind(k)).copied().collect()
    }
    fn keys<'a>(ls: &[(&'a str, &'a str)]) -> BTreeSet<&'a str> {
        ls.iter().filter(|(k, _)| !is_bind(k)).map(|(k, _)| *k).collect()
    }
    // Klavye düzeni/varyant/seçenek değiştiyse: tek tek `keyword` arada geçersiz bir xkb
    // bileşimi kurabilir ve boş değer `keyword`'e girmez; dosyadan birlikte okunur.
    fn layout<'a>(ls: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        ls.iter().filter(|(k, _)| is_reload_key(k)).copied().collect()
    }
    if binds(&old_lines) != binds(&new_lines)
        || !keys(&old_lines).is_subset(&keys(&new_lines))
        || layout(&old_lines) != layout(&new_lines)
    {
        return ApplyAction::Reload;
    }
    batch(new)
}

/// Değer batch'i; klavye düzeni anahtarları değişmediyse zaten etkindir, batch'e girmez.
fn batch(new: &Generated) -> ApplyAction {
    let kws: Vec<_> = new.keywords.iter().filter(|(k, _)| k != "exec-once" && !is_reload_key(k)).cloned().collect();
    if kws.is_empty() {
        return ApplyAction::Nothing;
    }
    keyword_batch(&kws).map_or(ApplyAction::Reload, ApplyAction::Keywords)
}

/// Adımı uygular, sonra `configerrors`'tan `names` (ör. `nuxut.conf`, `nuxut-base.conf`) dosyalarına ait
/// satırları döndürür (en fazla 8, kırpılmış).
pub fn apply(ctl: &dyn HyprControl, action: &ApplyAction, names: &[&str]) -> Result<Vec<String>, String> {
    match action {
        ApplyAction::Nothing => return Ok(Vec::new()),
        ApplyAction::Keywords(b) => {
            ctl.request(b)?;
        }
        ApplyAction::Reload => {
            // 0.56.2'de argüman yok sayılır (düz reload); eski sürümlerde monitörlere dokunmaz.
            ctl.request("reload config-only")?;
        }
        ApplyAction::FullReset => {
            ctl.request("reload full-reset")?;
        }
        ApplyAction::Eval(code) => {
            // Kod yalnızca üreticiden (lua.rs, kaçışlı); yine de tek satır ve sınırlı olmalı.
            if code.contains(['\n', '\r', '\0']) || code.len() > MAX_EVAL_BYTES {
                return Err("eval kodu geçersiz (çok satırlı ya da çok büyük)".into());
            }
            let reply = ctl.request(&format!("eval {code}"))?;
            if reply.trim() != "ok" {
                return Err(reply.chars().filter(|c| !c.is_control()).take(200).collect());
            }
        }
    }
    let errors = ctl.request("configerrors")?;
    Ok(errors
        .lines()
        .filter(|l| names.iter().any(|n| l.contains(n)))
        .take(8)
        .map(|l| l.chars().filter(|c| !c.is_control()).take(200).collect())
        .collect())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct Fake {
        sent: RefCell<Vec<String>>,
        errors: String,
    }

    impl HyprControl for Fake {
        fn request(&self, command: &str) -> Result<String, String> {
            self.sent.borrow_mut().push(command.to_owned());
            Ok(if command == "configerrors" { self.errors.clone() } else { "ok".into() })
        }
    }

    fn gen_(text: &str, kws: &[(&str, &str)]) -> Generated {
        Generated { text: text.into(), keywords: kws.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(), eval: None }
    }

    #[test]
    fn batch_is_safe() {
        let kw = |k: &str, v: &str| vec![(k.to_owned(), v.to_owned())];
        assert_eq!(keyword_batch(&kw("decoration:rounding", "22")).unwrap(), "[[BATCH]]keyword decoration:rounding 22");
        assert_eq!(keyword_batch(&kw("a", "1;reload")), None);
        assert_eq!(keyword_batch(&kw("a b", "1")), None);
        assert_eq!(keyword_batch(&kw("general:col.active_border", "rgba(ff0000ff)")).unwrap().matches(';').count(), 0);
    }

    #[test]
    fn plans() {
        let a = gen_("x = 1\nbind = SUPER, n, exec, a\n", &[("x", "1")]);
        let b = gen_("x = 2\nbind = SUPER, n, exec, a\n", &[("x", "2")]);
        let c = gen_("x = 2\n", &[("x", "2")]);
        let d = gen_("y = 2\nx = 2\nbind = SUPER, n, exec, a\n", &[("y", "2"), ("x", "2")]);
        assert_eq!(plan_apply(Some(&a.text), &a), ApplyAction::Nothing);
        assert_eq!(plan_apply(Some(&a.text), &b), ApplyAction::Keywords("[[BATCH]]keyword x 2".into()));
        // Bind kalktı → yeniden okuma.
        assert_eq!(plan_apply(Some(&b.text), &c), ApplyAction::Reload);
        // Anahtar eklendi: batch yeter; anahtar kalktı: yeniden okuma.
        assert!(matches!(plan_apply(Some(&b.text), &d), ApplyAction::Keywords(_)));
        assert_eq!(plan_apply(Some(&d.text), &b), ApplyAction::Reload);
        assert_eq!(plan_apply(None, &a), ApplyAction::Reload);
        assert!(matches!(plan_apply(None, &c), ApplyAction::Keywords(_)));
    }

    /// Klavye düzeni yeniden okumayla; diğer girdi değerleri batch ile; kaldırılan anahtar
    /// (Ayarlar'da "varsayılana dön") yeniden okuma → kullanıcının değeri geri gelir.
    #[test]
    fn input_plans() {
        let a = gen_("input:repeat_rate = 25\n", &[("input:repeat_rate", "25")]);
        let b = gen_(
            "input:kb_layout = tr,us\ninput:kb_variant = ,intl\ninput:kb_options = \ninput:repeat_rate = 30\n",
            &[("input:kb_layout", "tr,us"), ("input:kb_variant", ",intl"), ("input:kb_options", ""), ("input:repeat_rate", "30")],
        );
        let c = gen_(
            "input:kb_layout = tr,us\ninput:kb_variant = ,intl\ninput:kb_options = \ninput:repeat_rate = 40\n",
            &[("input:kb_layout", "tr,us"), ("input:kb_variant", ",intl"), ("input:kb_options", ""), ("input:repeat_rate", "40")],
        );
        assert_eq!(plan_apply(None, &a), ApplyAction::Keywords("[[BATCH]]keyword input:repeat_rate 25".into()));
        assert_eq!(plan_apply(None, &b), ApplyAction::Reload, "ilk düzen");
        assert_eq!(plan_apply(Some(&a.text), &b), ApplyAction::Reload, "düzen eklendi");
        assert_eq!(plan_apply(Some(&b.text), &c), ApplyAction::Keywords("[[BATCH]]keyword input:repeat_rate 40".into()), "düzen aynı, boş kb_options batch'e girmez");
        assert_eq!(plan_apply(Some(&c.text), &a), ApplyAction::Reload, "anahtar kalktı");
    }

    #[test]
    fn apply_with_fake() {
        let fake = Fake { errors: "Config error in file /h/nuxut.conf at line 3: x\nConfig error in file /h/other.conf\n".into(), ..Default::default() };
        let errs = apply(&fake, &ApplyAction::Keywords("[[BATCH]]keyword x 2".into()), &["nuxut.conf"]).unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(*fake.sent.borrow(), ["[[BATCH]]keyword x 2", "configerrors"]);
        apply(&fake, &ApplyAction::Reload, &["nuxut.conf"]).unwrap();
        assert_eq!(fake.sent.borrow()[2], "reload config-only");
        let n = fake.sent.borrow().len();
        apply(&fake, &ApplyAction::Nothing, &["nuxut.conf"]).unwrap();
        assert_eq!(fake.sent.borrow().len(), n);
    }
}

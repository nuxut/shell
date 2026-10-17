//! Pano teklifinin gizlilik kararı (saf; tasarım: `docs/design/clipboard.md` "Gizlilik kuralları").
//!
//! Karar yalnızca **MIME listesine** ve oturum bağlamına bakılarak verilir; içerik okunmaz.
//! Hassas teklif ([`Decision::Skip`]) için kaynak hiç okunmaz, önceki öğelere dokunulmaz.
//!
//! Sıra: özellik kapalı → kendi koyduğumuz seçim → oturum kilitli → bozuk/aşırı ya da
//! framework'ün kırptığı liste (`truncated`) → framework `sensitive` işareti → parola
//! yöneticisi ipucu → adında `secret`/`password` geçen MIME → hariç tutulan etkin pencere →
//! desteklenen tür yok. İlk eşleşen kural kazanır.

/// Parola yöneticilerinin (KeePassXC, KDE) "bu teklif hassastır" işareti; değeri okunmaz.
pub const PASSWORD_HINT: &str = nuxut::PASSWORD_HINT_MIME;
/// Teklifte kabul edilen en çok MIME (framework de kırpar; fazlası şüpheli sayılır).
pub const MAX_MIMES: usize = 64;
/// MIME adının en çok uzunluğu (bayt).
pub const MAX_MIME_LEN: usize = 256;

/// Metin MIME'ları, tercih sırasıyla. İçerik yine UTF-8 olarak doğrulanır.
const TEXT_MIMES: &[&str] = &["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "STRING", "TEXT"];

/// Saklanan görsel türleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageMime {
    Png,
    Jpeg,
}

impl ImageMime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }

    /// Kodlu baytların başı bu türle uyuşuyor mu (çözmeden, imza ile).
    pub fn matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Jpeg => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        }
    }
}

/// Okunacak türün sınıfı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image(ImageMime),
}

/// Teklif neden atlandı (log için; içerik ya da MIME adı taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Disabled,
    Own,
    Locked,
    /// MIME listesi sınırı aştı ya da aşırı uzun ad içeriyor.
    Malformed,
    /// Framework teklifi hassas işaretledi (listeden düşen türler dahil; okunamaz).
    Sensitive,
    PasswordHint,
    SecretMime,
    ExcludedApp,
    Unsupported,
}

/// Karar: okunacak tek MIME ya da atlama nedeni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// `mime`: teklifteki adın kendisi (kaynağa bu ad ile sorulur).
    Accept { mime: String, kind: Kind },
    Skip(SkipReason),
}

/// Kararın bağlamı (teklif dışındaki her şey).
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    pub enabled: bool,
    pub images: bool,
    /// Oturum kilitli ya da kilit ekranı açık.
    pub locked: bool,
    /// Kopyalama anındaki etkin pencere sınıfı (Hyprland `activewindow`).
    pub active_class: Option<&'a str>,
    pub exclude_apps: &'a [String],
}

/// `mime` hassas mı: parola yöneticisi ipucu ya da adında `secret`/`password`. Kural tek
/// yerde, framework'te (`nuxut::is_sensitive_mime`; framework `sensitive` işareti ve VM pano
/// köprüsü de onu kullanır): kopyalar ayrışırsa bir yol hassas teklifi kaçırırdı.
fn sensitive(mime: &str) -> Option<SkipReason> {
    if mime.eq_ignore_ascii_case(PASSWORD_HINT) {
        return Some(SkipReason::PasswordHint);
    }
    nuxut::is_sensitive_mime(mime).then_some(SkipReason::SecretMime)
}

/// Etkin pencere sınıfı hariç tutulan uygulamalardan biri mi (büyük/küçük harf duyarsız; boş
/// sınıf hiçbirine uymaz). Geçmiş filtresi ve VM pano köprüsü aynı kuralı kullanır.
pub fn excluded(active_class: Option<&str>, exclude_apps: &[String]) -> bool {
    active_class.filter(|c| !c.is_empty()).is_some_and(|class| exclude_apps.iter().any(|a| a.eq_ignore_ascii_case(class)))
}

/// Framework'ün teklif işaretleri (`DataControlEvent::SelectionOffered`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    /// Seçimi bu shell koydu (geri okunmaz).
    pub own: bool,
    /// Parola/gizli işaretli tür sunuldu (framework kararı; listeden düşenler dahil).
    pub sensitive: bool,
    /// Sınırı aşan türler listeden düştü (liste eksik).
    pub truncated: bool,
}

impl Flags {
    pub const NONE: Self = Self { own: false, sensitive: false, truncated: false };
    pub const OWN: Self = Self { own: true, ..Self::NONE };
}

/// Teklifin kararı.
pub fn decide(mimes: &[String], flags: Flags, cx: &Context<'_>) -> Decision {
    if !cx.enabled {
        return Decision::Skip(SkipReason::Disabled);
    }
    if flags.own {
        return Decision::Skip(SkipReason::Own);
    }
    if cx.locked {
        return Decision::Skip(SkipReason::Locked);
    }
    // Kırpılmış liste ipucunu saklamış olabilir: sınırı aşan teklif hiç okunmaz.
    if flags.truncated || mimes.len() > MAX_MIMES || mimes.iter().any(|m| m.len() > MAX_MIME_LEN) {
        return Decision::Skip(SkipReason::Malformed);
    }
    if flags.sensitive {
        return Decision::Skip(SkipReason::Sensitive);
    }
    // İpucu, diğer hassas adlardan önce: neden daha açıklayıcı.
    if mimes.iter().any(|m| m.eq_ignore_ascii_case(PASSWORD_HINT)) {
        return Decision::Skip(SkipReason::PasswordHint);
    }
    if let Some(reason) = mimes.iter().find_map(|m| sensitive(m)) {
        return Decision::Skip(reason);
    }
    if excluded(cx.active_class, cx.exclude_apps) {
        return Decision::Skip(SkipReason::ExcludedApp);
    }
    for want in TEXT_MIMES {
        if let Some(m) = mimes.iter().find(|m| m.eq_ignore_ascii_case(want)) {
            return Decision::Accept { mime: m.clone(), kind: Kind::Text };
        }
    }
    if cx.images {
        for image in [ImageMime::Png, ImageMime::Jpeg] {
            if let Some(m) = mimes.iter().find(|m| m.eq_ignore_ascii_case(image.as_str())) {
                return Decision::Accept { mime: m.clone(), kind: Kind::Image(image) };
            }
        }
    }
    Decision::Skip(SkipReason::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mimes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    fn cx<'a>(exclude: &'a [String]) -> Context<'a> {
        Context { enabled: true, images: true, locked: false, active_class: None, exclude_apps: exclude }
    }

    fn skip(m: &[&str], flags: Flags, c: &Context<'_>) -> Option<SkipReason> {
        match decide(&mimes(m), flags, c) {
            Decision::Skip(r) => Some(r),
            Decision::Accept { .. } => None,
        }
    }

    #[test]
    fn text_preferred_over_image_and_utf8_first() {
        let none = [];
        let d = decide(&mimes(&["image/png", "text/plain", "text/plain;charset=utf-8", "text/html"]), Flags::NONE, &cx(&none));
        assert_eq!(d, Decision::Accept { mime: "text/plain;charset=utf-8".into(), kind: Kind::Text });
        let d = decide(&mimes(&["TEXT", "UTF8_STRING"]), Flags::NONE, &cx(&none));
        assert_eq!(d, Decision::Accept { mime: "UTF8_STRING".into(), kind: Kind::Text });
        // Büyük/küçük harf duyarsız; teklifteki ad korunur.
        let d = decide(&mimes(&["text/plain;charset=UTF-8"]), Flags::NONE, &cx(&none));
        assert_eq!(d, Decision::Accept { mime: "text/plain;charset=UTF-8".into(), kind: Kind::Text });
    }

    #[test]
    fn image_only_offer_and_images_switch() {
        let none = [];
        let d = decide(&mimes(&["text/html", "image/jpeg", "image/png"]), Flags::NONE, &cx(&none));
        assert_eq!(d, Decision::Accept { mime: "image/png".into(), kind: Kind::Image(ImageMime::Png) });
        let d = decide(&mimes(&["image/jpeg"]), Flags::NONE, &cx(&none));
        assert_eq!(d, Decision::Accept { mime: "image/jpeg".into(), kind: Kind::Image(ImageMime::Jpeg) });
        let c = Context { images: false, ..cx(&none) };
        assert_eq!(skip(&["image/png"], Flags::NONE, &c), Some(SkipReason::Unsupported));
        // Dosya listesi ve zengin metin ayrı öğe olmaz.
        assert_eq!(skip(&["text/uri-list", "text/html", "image/gif"], Flags::NONE, &cx(&none)), Some(SkipReason::Unsupported));
    }

    #[test]
    fn sensitive_offers_are_skipped() {
        let none = [];
        let c = cx(&none);
        assert_eq!(skip(&["text/plain", PASSWORD_HINT], Flags::NONE, &c), Some(SkipReason::PasswordHint));
        assert_eq!(skip(&["text/plain", "X-KDE-PASSWORDMANAGERHINT"], Flags::NONE, &c), Some(SkipReason::PasswordHint));
        assert_eq!(skip(&["application/x-nuxut-secret", "text/plain"], Flags::NONE, &c), Some(SkipReason::SecretMime));
        assert_eq!(skip(&["text/plain", "application/X-Password-Field"], Flags::NONE, &c), Some(SkipReason::SecretMime));
        assert_eq!(skip(&["text/plain", "x-SeCrEt"], Flags::NONE, &c), Some(SkipReason::SecretMime));
        // Benzer ama hassas olmayan ad kabul edilir.
        assert_eq!(skip(&["text/plain", "application/x-pass"], Flags::NONE, &c), None);
        // Ad kuralı framework'ünkiyle aynı (köprü ve framework `sensitive` işareti de onu kullanır).
        for m in [PASSWORD_HINT, "x-PASSWORD", "a/secretive", "text/html", "application/x-pass", "SeCrEt"] {
            assert_eq!(skip(&["text/plain", m], Flags::NONE, &c).is_some(), nuxut::is_sensitive_mime(m), "{m}");
        }
    }

    #[test]
    fn excluded_app_rule() {
        let apps = vec!["Bitwarden".to_owned()];
        assert!(excluded(Some("bitwarden"), &apps));
        assert!(!excluded(Some("firefox"), &apps));
        assert!(!excluded(Some(""), &apps));
        assert!(!excluded(None, &apps));
        assert!(!excluded(Some("bitwarden"), &[]));
    }

    #[test]
    fn framework_flags_skip_without_reading() {
        let none = [];
        let c = cx(&none);
        // Framework işareti tek başına yeter (işaret türü listeden düşmüş olabilir).
        let sensitive = Flags { sensitive: true, ..Flags::NONE };
        assert_eq!(skip(&["text/plain"], sensitive, &c), Some(SkipReason::Sensitive));
        let truncated = Flags { truncated: true, ..Flags::NONE };
        assert_eq!(skip(&["text/plain"], truncated, &c), Some(SkipReason::Malformed));
        // Kendi seçimimiz her şeyden önce (kilit dışı bağlam).
        let all = Flags { own: true, sensitive: true, truncated: true };
        assert_eq!(skip(&["text/plain"], all, &c), Some(SkipReason::Own));
    }

    #[test]
    fn context_rules_and_order() {
        let exclude = vec!["org.keepassxc.KeePassXC".to_owned(), "Bitwarden".to_owned()];
        let c = cx(&exclude);
        assert_eq!(skip(&["text/plain"], Flags::NONE, &Context { enabled: false, ..c }), Some(SkipReason::Disabled));
        assert_eq!(skip(&["text/plain"], Flags::OWN, &c), Some(SkipReason::Own));
        assert_eq!(skip(&["text/plain"], Flags::NONE, &Context { locked: true, ..c }), Some(SkipReason::Locked));
        // Kilit, hassas addan önce gelir (her ikisi de okumaz).
        assert_eq!(skip(&[PASSWORD_HINT], Flags::NONE, &Context { locked: true, ..c }), Some(SkipReason::Locked));
        assert_eq!(skip(&["text/plain"], Flags::NONE, &Context { active_class: Some("bitwarden"), ..c }), Some(SkipReason::ExcludedApp));
        assert_eq!(skip(&["text/plain"], Flags::NONE, &Context { active_class: Some("org.keepassxc.KeePassXC"), ..c }), Some(SkipReason::ExcludedApp));
        assert_eq!(skip(&["text/plain"], Flags::NONE, &Context { active_class: Some("firefox"), ..c }), None);
        assert_eq!(skip(&["text/plain"], Flags::NONE, &Context { active_class: Some(""), ..c }), None);
    }

    #[test]
    fn limits_on_mime_list() {
        let none = [];
        let c = cx(&none);
        let mut many: Vec<String> = (0..MAX_MIMES).map(|i| format!("x/{i}")).collect();
        many.push("text/plain".into());
        assert_eq!(decide(&many, Flags::NONE, &c), Decision::Skip(SkipReason::Malformed));
        many.remove(0);
        assert!(matches!(decide(&many, Flags::NONE, &c), Decision::Accept { .. }), "tam sınırda kabul");
        let long = "a".repeat(MAX_MIME_LEN + 1);
        assert_eq!(decide(&mimes(&["text/plain", &long]), Flags::NONE, &c), Decision::Skip(SkipReason::Malformed));
        let edge = "a".repeat(MAX_MIME_LEN);
        assert!(matches!(decide(&mimes(&["text/plain", &edge]), Flags::NONE, &c), Decision::Accept { .. }));
        assert_eq!(decide(&[], Flags::NONE, &c), Decision::Skip(SkipReason::Unsupported));
    }

    #[test]
    fn image_signatures() {
        assert!(ImageMime::Png.matches(b"\x89PNG\r\n\x1a\nrest"));
        assert!(!ImageMime::Png.matches(b"\xFF\xD8\xFF"));
        assert!(ImageMime::Jpeg.matches(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!ImageMime::Jpeg.matches(b"GIF89a"));
    }
}

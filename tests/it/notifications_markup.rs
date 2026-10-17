//! Bildirim gövdesi markup ayrıştırıcısı, bağlantı doğrulaması (atlatma vektörleri), hint ve
//! görsel doğrulama, güvenli dosya okuma (symlink/FIFO/aygıt/procfs) ve istek sınırları
//! (D-Bus gerektirmez).

use std::collections::HashMap;

use nuxut_shell::services::notifications::hints::{
    FileError, HintValue, Hints, ImageDataHint, ImageError, RawIcon, parse_icon_ref, read_image_file,
};
use nuxut_shell::services::notifications::markup::{is_safe_link, parse_markup, plain_body, validate_link};
use nuxut_shell::services::notifications::server::{MAX_BODY_BYTES, MAX_SUMMARY_CHARS, build_notification};
use nuxut_shell::services::notifications::{BodyLink, BodySpan, Expiry, NotifyIcon, TextStyle, Urgency, body_text};

fn span(text: &str, style: TextStyle, link: Option<(&str, &str)>) -> BodySpan {
    BodySpan { text: text.into(), style, link: link.map(|(url, host)| BodyLink { url: url.into(), host: host.into() }) }
}

const PLAIN: TextStyle = TextStyle { bold: false, italic: false, underline: false };
const BOLD: TextStyle = TextStyle { bold: true, italic: false, underline: false };

fn plain(markup: &str) -> String {
    body_text(&parse_markup(markup))
}

#[test]
fn spec_subset_becomes_spans() {
    assert_eq!(
        parse_markup("Merhaba <b>kalın <i>ikisi</i></b> <u>alt</u>"),
        vec![
            span("Merhaba ", PLAIN, None),
            span("kalın ", BOLD, None),
            span("ikisi", TextStyle { bold: true, italic: true, underline: false }, None),
            span(" ", PLAIN, None),
            span("alt", TextStyle { underline: true, ..PLAIN }, None),
        ]
    );
}

#[test]
fn links_only_safe_schemes() {
    let r = parse_markup(
        r#"<a href="https://example.org/x?a=1&amp;b=2">site</a> <a href='javascript:alert(1)'>kötü</a> <a href=mailto:a@b.c>posta</a>"#,
    );
    assert_eq!(r[0], span("site", PLAIN, Some(("https://example.org/x?a=1&b=2", "example.org"))));
    // Güvensiz bağlantı düşer, metin kalır (komşu düz metinle birleşir).
    assert_eq!(r[1], span(" kötü ", PLAIN, None));
    assert_eq!(r[2], span("posta", PLAIN, Some(("mailto:a@b.c", "a@b.c"))));

    // Normalleştirme: şema/host küçük harf, IDN punycode (görünen host hedefi yansıtır).
    let l = validate_link("HTTPS://ExAmPle.ORG/Yol").unwrap();
    assert_eq!((l.url.as_str(), l.host.as_str()), ("https://example.org/Yol", "example.org"));
    let idn = validate_link("https://bücher.example/").unwrap();
    assert_eq!(idn.host, "xn--bcher-kva.example");
    for ok in ["http://a.b", "https://a.b/c#d", "mailto:x@y.z", "https://1.2.3.4:8080/p", "https://[::1]/"] {
        assert!(is_safe_link(ok), "{ok}");
    }
}

#[test]
fn link_bypass_vectors_rejected() {
    let long = format!("https://example.org/{}", "a".repeat(3000));
    for bad in [
        // Şema allowlist dışı ve büyük/küçük harf oyunları.
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "vbscript:x",
        "data:text/html,<script>x</script>",
        "data:image/png;base64,AAAA",
        "file:///etc/passwd",
        "FILE:///etc/passwd",
        "ftp://a.b/",
        "x-scheme-handler://a",
        "smb://host/share",
        // Baştaki/aradaki boşluk ve kontrol karakterleri (kırpılmaz, reddedilir).
        "  JaVaScRiPt:alert(1)",
        " https://a.b",
        "\u{0}https://a.b",
        "https://a.b\u{0}",
        "\tjavascript:x",
        "java\nscript:x",
        "java\tscript:alert(1)",
        "https://a.b/\u{7f}",
        // Görünmez/bidi karakterler.
        "https://a.b/\u{202e}gpj.exe",
        "\u{200b}javascript:x",
        "https://exa\u{ad}mple.org",
        "\u{feff}https://a.b",
        // Ters bölü ve ayrıştırıcının "onardığı" biçimler.
        "https:/\\evil.example",
        "https:\\\\evil.example",
        "https://good.example\\@evil.example",
        "https:/evil.example",
        "https:///evil.example",
        "https:evil.example",
        // Kullanıcı bilgisiyle aldatma.
        "https://good.example@evil.example/",
        "https://good.example:pass@evil.example/",
        "https://:@evil.example/",
        // Host yok / bozuk.
        "http://",
        "https://",
        "https:// /",
        "https://exa mple.org",
        // mailto başlıkları ve adressiz.
        "mailto:",
        "mailto:a@b.c?attach=/etc/passwd",
        "mailto:a@b.c?body=x",
        "mailto:nobody",
        // Uzunluk.
        long.as_str(),
        // Şemasız / göreli.
        "",
        "x",
        "//evil.example/",
        "/etc/passwd",
    ] {
        assert!(validate_link(bad).is_none(), "kabul edilmemeliydi: {bad:?}");
    }
}

#[test]
fn entity_encoded_schemes_rejected_after_decoding() {
    // Entity çözümünden SONRA doğrulanır: kodlanmış şema da yakalanır, metin kalır.
    for markup in [
        r#"<a href="&#106;avascript:alert(1)">x</a>"#,
        r#"<a href="&#x6A;&#x61;vascript:alert(1)">x</a>"#,
        r#"<a href="javascript&#58;alert(1)">x</a>"#,
        r#"<a href="&#32;javascript:alert(1)">x</a>"#,
        r#"<a href="&#9;javascript:alert(1)">x</a>"#,
        r#"<a href="https:&#x2F;&#x5C;evil.example">x</a>"#,
        r#"<a href="https://good.example&#64;evil.example">x</a>"#,
        r#"<a href="&#x66;ile:///etc/passwd">x</a>"#,
        r#"<a href=" https://a.b">x</a>"#,
        r#"<a HREF="data:text/html,x">x</a>"#,
    ] {
        let r = parse_markup(markup);
        assert_eq!(r, vec![span("x", PLAIN, None)], "{markup}");
    }
    // Kodlanmış ama güvenli bağlantı çözülüp kabul edilir.
    let r = parse_markup(r#"<a href="https&#58;//example.org/&#97;">x</a>"#);
    assert_eq!(r, vec![span("x", PLAIN, Some(("https://example.org/a", "example.org")))]);
}

#[test]
fn deceptive_text_exposes_real_host() {
    let r = parse_markup(r#"<a href="https://evil.example/login">https://bank.example</a>"#);
    let link = r[0].link.as_ref().unwrap();
    assert_eq!(r[0].text, "https://bank.example");
    assert_eq!(link.host, "evil.example");
}

#[test]
fn img_src_never_becomes_link_or_path() {
    let r = parse_markup(r#"<img src="https://tracker.example/p.png" alt="logo"/><img src="file:///etc/passwd"/>"#);
    assert_eq!(r, vec![span("logo", PLAIN, None)]);
}

#[test]
fn unknown_tags_stripped_entities_decoded() {
    let r = parse_markup("<span foreground='red'>kırmızı</span> <script>x</script> &lt;3 &amp; &#65;&#x42; &bogus; &");
    assert_eq!(body_text(&r), "kırmızı x <3 & AB &bogus; &");
    assert_eq!(r.len(), 1);
}

#[test]
fn img_alt_and_br_and_comments() {
    assert_eq!(
        plain(r#"önce<img src="/x.png" alt="[resim &amp; alt]"/>sonra<br/>yeni<!-- gizli -->satır"#),
        "önce[resim & alt]sonra\nyenisatır"
    );
}

#[test]
fn malformed_input_is_text() {
    assert_eq!(plain("a < b > c"), "a < b > c");
    assert_eq!(plain("<3 seni"), "<3 seni");
    assert_eq!(plain("kapanmayan <b"), "kapanmayan <b");
    // Kapanmayan/fazla kapanan biçim sayaçları taşmaz.
    assert!(parse_markup(&"<b>".repeat(500)).is_empty());
    assert_eq!(parse_markup("</b></b>x<b>y"), vec![span("x", PLAIN, None), span("y", BOLD, None)]);
    // Kontrol karakterleri atılır, satır sonu kalır.
    assert_eq!(plain("a\u{7}b\r\nc\u{0}"), "ab\nc");
    assert_eq!(body_text(&plain_body("x\u{1b}[31my")), "x[31my");
}

fn pixmap(icon: &RawIcon) -> (u32, u32, &[u8]) {
    match icon {
        RawIcon::Pixmap { width, height, rgba } => (*width, *height, &rgba[..]),
        other => panic!("Pixmap değil: {other:?}"),
    }
}

#[test]
fn image_data_validation() {
    let rgb = ImageDataHint {
        width: 2,
        height: 2,
        rowstride: 8, // 6 + 2 dolgu
        has_alpha: false,
        bits_per_sample: 8,
        channels: 3,
        data: vec![1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12], // son satır dolgusuz
    };
    let img = rgb.decode().expect("geçerli");
    assert_eq!(pixmap(&img), (2, 2, &[1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255][..]));

    let rgba = ImageDataHint {
        width: 1,
        height: 1,
        rowstride: 4,
        has_alpha: true,
        bits_per_sample: 8,
        channels: 4,
        data: vec![9, 8, 7, 6],
    };
    assert_eq!(pixmap(&rgba.decode().unwrap()).2, &[9, 8, 7, 6]);

    let bad = |f: fn(&mut ImageDataHint)| {
        let mut h = rgba.clone();
        f(&mut h);
        h.decode().unwrap_err()
    };
    assert!(matches!(bad(|h| h.width = 0), ImageError::Size(..)));
    assert!(matches!(bad(|h| h.width = 513), ImageError::Size(..)));
    assert!(matches!(bad(|h| h.height = -5), ImageError::Size(..)));
    assert!(matches!(bad(|h| h.bits_per_sample = 16), ImageError::Format { .. }));
    assert!(matches!(bad(|h| h.channels = 3), ImageError::Format { .. }));
    assert!(matches!(bad(|h| h.rowstride = 3), ImageError::Rowstride { .. }));
    assert!(matches!(bad(|h| h.rowstride = -4), ImageError::Rowstride { .. }));
    assert!(matches!(bad(|h| h.rowstride = i32::MAX), ImageError::Rowstride { .. }));
    assert!(matches!(bad(|h| h.data.pop().map_or((), drop)), ImageError::Length { .. }));
    assert!(matches!(bad(|h| h.data.extend([0; 100])), ImageError::Length { .. }));
    // 512 sınırı dahil.
    let big = ImageDataHint {
        width: 512,
        height: 512,
        rowstride: 2048,
        has_alpha: true,
        bits_per_sample: 8,
        channels: 4,
        data: vec![0; 2048 * 512],
    };
    assert!(big.decode().is_ok());
}

fn encoded(bytes: &[u8], name: &str) -> RawIcon {
    RawIcon::Encoded { bytes: bytes.into(), file_name: name.into() }
}

#[test]
fn icon_refs_local_only() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a b.png");
    std::fs::write(&file, b"PNGDATA").unwrap();
    let uri = format!("file://{}", dir.path().join("a%20b.png").display());

    assert_eq!(parse_icon_ref("dialog-information"), RawIcon::Name("dialog-information".into()));
    // Yol değil, okunmuş baytlar modele girer.
    assert_eq!(parse_icon_ref(file.to_str().unwrap()), encoded(b"PNGDATA", "a b.png"));
    assert_eq!(parse_icon_ref(&uri), encoded(b"PNGDATA", "a b.png"));
    assert_eq!(parse_icon_ref(&format!("file://localhost{}", file.display())), encoded(b"PNGDATA", "a b.png"));
    let dotdot = format!("{}/../{}/a b.png", dir.path().display(), dir.path().file_name().unwrap().to_str().unwrap());
    for bad in [
        "",
        "https://example.org/a.png",
        "smb://host/a.png",
        "file://host/a.png",
        "../a.png",
        "rel/a.png",
        "/nonexistent/x.png",
        "file:///tmp/%00x",
        dotdot.as_str(),
    ] {
        assert_eq!(parse_icon_ref(bad), RawIcon::None, "{bad}");
    }
    // Dizin de reddedilir.
    assert_eq!(parse_icon_ref(dir.path().to_str().unwrap()), RawIcon::None);
}

#[test]
fn image_file_attacks_rejected() {
    use std::os::unix::fs::symlink;
    use std::path::Path;

    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.png");
    std::fs::write(&real, b"gercek").unwrap();
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, b"gizli").unwrap();

    // Son bileşen symlink: izlenmez.
    let link = dir.path().join("link.png");
    symlink(&secret, &link).unwrap();
    assert!(matches!(read_image_file(&link), Err(FileError::NotRegular)));
    // file:// ile de.
    assert_eq!(parse_icon_ref(&format!("file://{}", link.display())), RawIcon::None);

    // Symlink değişimi (TOCTOU): doğrulama ve okuma tek açılışta; sonradan dosyayı symlink'le
    // değiştirmek modeldeki baytları etkilemez, model yolu tutmaz.
    let swapped = dir.path().join("swap.png");
    std::fs::write(&swapped, b"ilk").unwrap();
    let icon = read_image_file(&swapped).unwrap();
    std::fs::remove_file(&swapped).unwrap();
    symlink(&secret, &swapped).unwrap();
    assert_eq!(icon, encoded(b"ilk", "swap.png"));
    assert!(matches!(read_image_file(&swapped), Err(FileError::NotRegular)));

    // FIFO: açılış beklemez (O_NONBLOCK), normal dosya değil diye reddedilir.
    let fifo = dir.path().join("pipe.png");
    rustix::fs::mknodat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        0,
    )
    .unwrap();
    let started = std::time::Instant::now();
    assert!(matches!(read_image_file(&fifo), Err(FileError::NotRegular)));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));

    // Aygıt ve sahte dosya sistemleri (önek).
    for dev in
        ["/dev/null", "/dev/zero", "/dev/urandom", "/proc/self/environ", "/proc/self/mem", "/sys/kernel/uevent_seqnum"]
    {
        assert!(matches!(read_image_file(Path::new(dev)), Err(FileError::PseudoFs)), "{dev}");
    }
    // Önek kontrolünü ara dizin symlink'iyle aşmak: fstatfs yakalar (procfs).
    let proc_link = dir.path().join("p");
    symlink("/proc/self", &proc_link).unwrap();
    assert!(matches!(read_image_file(&proc_link.join("environ")), Err(FileError::PseudoFs)));
    let sys_link = dir.path().join("s");
    symlink("/sys/kernel", &sys_link).unwrap();
    assert!(matches!(read_image_file(&sys_link.join("uevent_seqnum")), Err(FileError::PseudoFs)));
    // Ara dizin symlink'iyle aygıta: aygıt normal dosya değil.
    let dev_link = dir.path().join("d");
    symlink("/dev", &dev_link).unwrap();
    assert!(matches!(read_image_file(&dev_link.join("zero")), Err(FileError::NotRegular)));

    // Göreli, `..`, boş dosya, sınırdan büyük dosya.
    assert!(matches!(read_image_file(Path::new("real.png")), Err(FileError::Path)));
    assert!(matches!(read_image_file(&dir.path().join("x/../real.png")), Err(FileError::Path)));
    let empty = dir.path().join("empty.png");
    std::fs::write(&empty, b"").unwrap();
    assert!(matches!(read_image_file(&empty), Err(FileError::Size)));
    let big = dir.path().join("big.png");
    std::fs::File::create(&big).unwrap().set_len(16 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(read_image_file(&big), Err(FileError::Size)));
    // Ara dizin symlink'i olan meşru yol kabul edilir.
    let alias = dir.path().join("alias");
    symlink(dir.path(), &alias).unwrap();
    assert_eq!(read_image_file(&alias.join("real.png")).unwrap(), encoded(b"gercek", "real.png"));
}

#[test]
fn hints_parse_and_priority() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("img.png");
    std::fs::write(&file, b"x").unwrap();
    let pixel = ImageDataHint {
        width: 1,
        height: 1,
        rowstride: 4,
        has_alpha: true,
        bits_per_sample: 8,
        channels: 4,
        data: vec![1, 2, 3, 4],
    };

    let mut map = HashMap::new();
    map.insert("urgency".to_owned(), HintValue::Int(2));
    map.insert("category".to_owned(), HintValue::Str("im.received".into()));
    map.insert("desktop-entry".to_owned(), HintValue::Str("org.example.Chat".into()));
    map.insert("resident".to_owned(), HintValue::Bool(true));
    map.insert("transient".to_owned(), HintValue::Int(1));
    map.insert("x".to_owned(), HintValue::Int(10));
    map.insert("image-path".to_owned(), HintValue::Str(file.to_str().unwrap().into()));
    let h = Hints::parse(&map);
    assert_eq!(h.urgency, Urgency::Critical);
    assert_eq!(h.category.as_deref(), Some("im.received"));
    assert_eq!(h.desktop_entry.as_deref(), Some("org.example.Chat"));
    assert!(h.resident && h.transient && !h.suppress_sound);
    assert_eq!(h.image, encoded(b"x", "img.png"));

    // image-data, image-path'ten önce gelir; bozuk image-data path'e düşer.
    map.insert("image-data".to_owned(), HintValue::Image(pixel.clone()));
    assert!(matches!(Hints::parse(&map).image, RawIcon::Pixmap { .. }));
    map.insert("image-data".to_owned(), HintValue::Image(ImageDataHint { width: 5000, ..pixel }));
    assert_eq!(Hints::parse(&map).image, encoded(b"x", "img.png"));

    map.insert("urgency".to_owned(), HintValue::Int(0));
    assert_eq!(Hints::parse(&map).urgency, Urgency::Low);
    map.insert("urgency".to_owned(), HintValue::Int(77));
    assert_eq!(Hints::parse(&map).urgency, Urgency::Normal);
}

#[test]
fn notification_limits_and_actions() {
    let long_summary = "ş".repeat(2000);
    let long_body = format!("{}<b>son</b>", "a".repeat(MAX_BODY_BYTES - 3));
    let actions: Vec<String> = ["default", "Aç", "", "boş anahtar", "reply", "Yanıtla", "reply", "tekrar", "tek"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let n = build_notification("Uyg\nAdı", "", &long_summary, &long_body, &actions, &HashMap::new(), -1).0;
    assert_eq!(n.app_name, "Uyg Adı");
    assert_eq!(n.summary.chars().count(), MAX_SUMMARY_CHARS);
    // Yarım kalan `<b` atıldı; metin sınırın altında.
    assert_eq!(body_text(&n.body), "a".repeat(MAX_BODY_BYTES - 3));
    assert!(n.has_default_action);
    let keys: Vec<_> = n.actions.iter().map(|a| a.key.as_str()).collect();
    assert_eq!(keys, ["reply"]);
    assert_eq!(n.expire, Expiry::Default);
    assert_eq!(n.app_icon, NotifyIcon::None);

    let many: Vec<String> = (0..100).flat_map(|i| [format!("k{i}"), format!("L{i}")]).collect();
    let n = build_notification("", "", "", "", &many, &HashMap::new(), 0).0;
    assert_eq!(n.actions.len(), 8);
    assert!(!n.has_default_action);
    assert_eq!(n.expire, Expiry::Never);
    assert_eq!(
        build_notification("", "", "", "", &[], &HashMap::new(), 1500).0.expire,
        Expiry::After(std::time::Duration::from_millis(1500))
    );
}

#[test]
fn encoded_images_never_enter_model() {
    use nuxut_shell::services::notifications::images::{ImageSource, Slot};

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.svg");
    std::fs::write(&file, b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'/>").unwrap();
    let mut map = HashMap::new();
    map.insert("image-path".to_owned(), HintValue::Str(file.to_str().unwrap().into()));
    let (n, jobs) = build_notification("a", file.to_str().unwrap(), "", "", &[], &map, -1);
    // Bildirimde yer tutucu; baytlar yalnızca çözme işinde.
    assert_eq!((n.app_icon.clone(), n.image.clone()), (NotifyIcon::None, NotifyIcon::None));
    let slots: Vec<Slot> = jobs.iter().map(|j| j.slot).collect();
    assert_eq!(slots, [Slot::AppIcon, Slot::Image]);
    assert!(jobs.iter().all(|j| matches!(&j.source, ImageSource::Bytes(b) if !b.is_empty())));
    // Tema adı: modelde ad (yer tutucu), çözme işi temada arar.
    let (n, jobs) = build_notification("a", "firefox", "", "", &[], &HashMap::new(), -1);
    assert_eq!(n.app_icon, NotifyIcon::Name("firefox".into()));
    assert_eq!(
        jobs.iter().map(|j| (j.slot, j.source.clone())).collect::<Vec<_>>(),
        [(Slot::AppIcon, ImageSource::IconName("firefox".into()))]
    );
    // app_icon yoksa masaüstü girdisi (dosya bulunamazsa girdi adı) denenir.
    let mut map = HashMap::new();
    map.insert("desktop-entry".to_owned(), HintValue::Str("org.example.Yok".into()));
    let (n, jobs) = build_notification("a", "", "", "", &[], &map, -1);
    assert_eq!(n.app_icon, NotifyIcon::Name("org.example.Yok".into()));
    assert_eq!(jobs.len(), 1);
    // image-data hemen hazır, iş yok.
    let pixel = ImageDataHint {
        width: 1,
        height: 1,
        rowstride: 4,
        has_alpha: true,
        bits_per_sample: 8,
        channels: 4,
        data: vec![1, 2, 3, 4],
    };
    let mut map = HashMap::new();
    map.insert("image-data".to_owned(), HintValue::Image(pixel));
    let (n, jobs) = build_notification("a", "", "", "", &[], &map, -1);
    assert!(matches!(n.image, NotifyIcon::Pixmap { .. }));
    assert!(jobs.is_empty());
}

#[test]
fn app_icon_side_follows_output_scale() {
    use nuxut_shell::services::notifications::images::{APP_ICON_MAX_SIDE, Slot, app_icon_side};

    assert_eq!(app_icon_side(1.0), 64);
    assert_eq!(app_icon_side(1.25), 80);
    assert_eq!(app_icon_side(1.5), 96);
    assert_eq!(app_icon_side(2.0), 128);
    assert_eq!(app_icon_side(4.0), APP_ICON_MAX_SIDE);
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(app_icon_side(bad), 64, "{bad}");
    }
    // İçerik görseli ölçekten bağımsız 256.
    assert_eq!(Slot::Image.max_side(64), 256);
    assert_eq!(Slot::AppIcon.max_side(96), 96);
    assert_eq!(Slot::AppIcon.max_side(500), APP_ICON_MAX_SIDE);
}

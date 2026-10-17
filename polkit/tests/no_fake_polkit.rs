//! Test kancaları derleme zamanı kapısının arkasında: `fake-polkit` olmadan derlenen ikilide
//! sahte bus/yardımcı/başsız diyalog kodu yok (ortam değişkeni adları bile girmemiş). Sürüm
//! derlemesinde özellik açılamaz (`main.rs` `compile_error`).

#[test]
#[cfg(not(feature = "fake-polkit"))]
fn binary_has_no_fake_polkit_code() {
    let bin = std::fs::read(env!("CARGO_BIN_EXE_nuxut-polkit")).expect("nuxut-polkit ikilisi okunamadı");
    for needle in [
        &b"NUXUT_POLKIT_FAKE_BUS"[..],
        b"NUXUT_POLKIT_FAKE_HELPER_SOCKET",
        b"NUXUT_POLKIT_FAKE_HEADLESS",
        b"NUXUT_POLKIT_TEST_PASSWORDS",
        b"--headless",
    ] {
        assert!(
            !bin.windows(needle.len()).any(|w| w == needle),
            "fake-polkit kapalıyken ikilide {} izi var",
            String::from_utf8_lossy(needle)
        );
    }
}

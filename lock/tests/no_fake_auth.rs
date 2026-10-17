//! Sahte doğrulayıcı derleme zamanı kapısının arkasında: `fake-auth` özelliği olmadan derlenen
//! ikilide sahte kipin ortam değişkeni adı bile yok (kod hiç girmemiş). Sürüm derlemesinde özellik
//! açılamaz (`main.rs` `compile_error`); `NUXUT_LOCK_INSTANCE` bu kapıya yalnızca ek bir şarttır.
//! Aynı kapı canlı test kancasını (evre bekletme) da kapsar.

#[test]
#[cfg(not(feature = "fake-auth"))]
fn binary_has_no_fake_auth_code() {
    let bin = std::fs::read(env!("CARGO_BIN_EXE_nuxut-lock")).expect("nuxut-lock ikilisi okunamadı");
    for needle in [
        &b"NUXUT_LOCK_FAKE_SECRET"[..],
        b"NUXUT_LOCK_TEST_HOLD_PENDING_MS",
        b"NUXUT_LOCK_TEST_HOLD_DONE_MS",
    ] {
        assert!(
            !bin.windows(needle.len()).any(|w| w == needle),
            "fake-auth özelliği kapalıyken ikilide {} izi var",
            String::from_utf8_lossy(needle)
        );
    }
}

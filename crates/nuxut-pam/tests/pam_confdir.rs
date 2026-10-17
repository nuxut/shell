//! Gerçek libpam ile uçtan uca: geçici dizindeki servis dosyaları (`pam_start_confdir`).
//! Gerçek kullanıcı şifresi kullanılmaz; `pam_exec expose_authtok` sabit test şifresini
//! conversation üzerinden alıp karşılaştırır.
//!
//! `cargo test -p nuxut-pam --features testing`

use std::path::{Path, PathBuf};

use nuxut_pam::{MessageKind, Outcome, SecretBuf, Service, authenticate, current_user};

const TEST_PASSWORD: &str = "nuxut-test-sifresi";

/// Fikstür yolu ÇALIŞMA ZAMANINDA çözülür (`cargo test` `CARGO_MANIFEST_DIR`'i test sürecine
/// verir); derleme anındaki `env!` yalnızca ikili cargo dışında çalıştırılınca yedektir.
///
/// Neden: cargo, paketin mutlak yolu değişince test ikilisini yeniden derlemez (meta karması
/// yolu içermez). İkili başka bir yoldan (`--manifest-path <sembolik bağlı kum havuzu>`)
/// derlenip o yol silinince `env!` ölü yolu gösterir; pam_exec `execve` ENOENT alır ve
/// `PAM_SYSTEM_ERR` (Error(4)) döner — "ara sıra" düşen test buydu (journal: 2026-10-14).
fn fixture(name: &str) -> PathBuf {
    let root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let path = root.join("tests/fixtures").join(name);
    assert!(
        path.is_file(),
        "fikstür yok: {} (ikili silinmiş bir yoldan mı derlendi?)",
        path.display()
    );
    path
}

/// `dir/<name>` servis dosyası yazar ve o servisi döndürür.
fn service(dir: &Path, name: &str, body: &str) -> Service {
    std::fs::write(dir.join(name), body).unwrap();
    Service::with_confdir(name, dir).unwrap()
}

fn secret(s: &str) -> SecretBuf {
    let mut b = SecretBuf::new();
    b.push_str(s).unwrap();
    b
}

fn user() -> String {
    current_user().unwrap().name
}

#[test]
fn permit_and_deny() {
    let dir = tempfile::tempdir().unwrap();
    let permit = service(dir.path(), "permit", "auth required pam_permit.so\n");
    assert_eq!(
        authenticate(&permit, &user(), &secret("x")).outcome,
        Outcome::Success
    );
    let deny = service(dir.path(), "deny", "auth required pam_deny.so\n");
    let r = authenticate(&deny, &user(), &secret("x"));
    assert!(!r.outcome.is_success());
    assert_eq!(r.outcome, Outcome::Denied);
}

#[test]
fn password_goes_through_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "auth required pam_exec.so quiet expose_authtok {}\n",
        fixture("check-password.sh").display()
    );
    let svc = service(dir.path(), "exec", &body);
    assert_eq!(
        authenticate(&svc, &user(), &secret(TEST_PASSWORD)).outcome,
        Outcome::Success
    );

    for wrong in [
        "",
        "nuxut-test-sifres",
        "nuxut-test-sifresi2",
        "NUXUT-TEST-SIFRESI",
    ] {
        let r = authenticate(&svc, &user(), &secret(wrong));
        assert!(!r.outcome.is_success(), "{wrong:?} açmamalı");
        assert!(r.messages.iter().all(|m| !m.text.contains(TEST_PASSWORD)));
    }
}

#[test]
fn module_messages_reach_the_ui() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(
        dir.path(),
        "echo",
        "auth optional pam_echo.so Hesap kilitli\nauth required pam_deny.so\n",
    );
    let r = authenticate(&svc, &user(), &secret("x"));
    assert_eq!(r.outcome, Outcome::Denied);
    assert_eq!(r.messages.len(), 1);
    assert_eq!(r.messages[0].kind, MessageKind::Info);
    assert_eq!(r.messages[0].text, "Hesap kilitli");
}

#[test]
fn broken_stacks_never_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("missing-module", "auth required pam_nuxut_yok.so\n"),
        ("empty", ""),
        ("garbage", "this is not a pam file\n"),
        // Yalnızca optional modüller başarısızsa da başarı olmamalı.
        ("optional-deny", "auth optional pam_deny.so\n"),
    ];
    for (name, body) in cases {
        let svc = service(dir.path(), name, body);
        let r = authenticate(&svc, &user(), &secret(TEST_PASSWORD));
        assert!(!r.outcome.is_success(), "{name}: {:?}", r.outcome);
    }
    // Servis dosyası yok (confdir'de `other` de yok).
    let svc = Service::with_confdir("yok", dir.path()).unwrap();
    assert!(
        !authenticate(&svc, &user(), &secret(TEST_PASSWORD))
            .outcome
            .is_success()
    );
}

#[test]
fn unknown_user_is_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "auth required pam_exec.so quiet expose_authtok {}\n",
        fixture("check-password.sh").display()
    );
    let svc = service(dir.path(), "exec", &body);
    // pam_exec kullanıcıyı denetlemez; şifre doğruysa açar. Bu, servis dosyasının (login
    // yığını) işidir — burada yalnızca NUL'lu adın PAM'e ulaşmadığını doğrularız.
    assert!(
        !authenticate(&svc, "kök\0", &secret(TEST_PASSWORD))
            .outcome
            .is_success()
    );
}

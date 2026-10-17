//! Doğrulama: `Authenticator` arkasında PAM (testte sahte), her deneme kendi thread'inde.
//!
//! Şifre [`SecretBuf`] olarak thread'e **taşınır** ve orada düşer (sıfırlanır); UI thread'inde
//! kopya kalmaz. Thread panik ederse ya da başlatılamazsa sonuç `None` olur (= başarısız).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use nuxut_pam::{AuthResult, SecretBuf, Service};

use crate::state::AttemptId;

/// Bloklayan doğrulayıcı. Birden çok thread'den çağrılabilir.
pub trait Authenticator: Send + Sync + 'static {
    fn authenticate(&self, secret: &SecretBuf) -> AuthResult;
}

/// Gerçek PAM: servis kilitlemeden önce seçilir ([`Service::system`]).
#[derive(Debug)]
pub struct PamAuthenticator {
    service: Service,
    user: String,
}

impl PamAuthenticator {
    pub fn new(service: Service, user: String) -> Self {
        Self { service, user }
    }

    pub fn service(&self) -> &Service {
        &self.service
    }
}

impl Authenticator for PamAuthenticator {
    fn authenticate(&self, secret: &SecretBuf) -> AuthResult {
        nuxut_pam::authenticate(&self.service, &self.user, secret)
    }
}

/// Denemeyi ayrı thread'de çalıştırır; bitince `done(id, sonuç)` çağrılır (o thread'den).
/// Thread yaratılamazsa `done(id, None)` hemen bu thread'de çağrılır.
pub fn spawn_attempt<A: Authenticator + ?Sized>(
    auth: Arc<A>,
    id: AttemptId,
    secret: SecretBuf,
    done: impl FnOnce(AttemptId, Option<AuthResult>) + Send + 'static,
) {
    // `done` iki yoldan birinde çağrılır; thread başlatılamazsa geri almak için paylaşılır.
    let slot = Arc::new(std::sync::Mutex::new(Some(done)));
    let thread_slot = Arc::clone(&slot);
    let spawned = std::thread::Builder::new()
        .name("nuxut-auth".into())
        .spawn(move || {
            let result = catch_unwind(AssertUnwindSafe(|| auth.authenticate(&secret))).ok();
            drop(secret);
            if let Some(done) = thread_slot.lock().ok().and_then(|mut s| s.take()) {
                done(id, result);
            }
        });
    if spawned.is_err()
        && let Some(done) = slot.lock().ok().and_then(|mut s| s.take())
    {
        done(id, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_pam::Outcome;
    use std::sync::mpsc;
    use std::time::Duration;

    struct Fake {
        expected: &'static [u8],
        seen: std::sync::Mutex<Vec<Vec<u8>>>,
    }

    impl Authenticator for Fake {
        fn authenticate(&self, secret: &SecretBuf) -> AuthResult {
            self.seen.lock().unwrap().push(secret.as_bytes().to_vec());
            let outcome = if secret.as_bytes() == self.expected {
                Outcome::Success
            } else {
                Outcome::Denied
            };
            AuthResult {
                outcome,
                messages: vec![],
            }
        }
    }

    struct Panics;
    impl Authenticator for Panics {
        fn authenticate(&self, _: &SecretBuf) -> AuthResult {
            panic!("modül çöktü");
        }
    }

    fn run<A: Authenticator>(auth: Arc<A>, pw: &str) -> (AttemptId, Option<AuthResult>) {
        let (tx, rx) = mpsc::channel();
        let mut s = SecretBuf::new();
        s.push_str(pw).unwrap();
        spawn_attempt(auth, 7, s, move |id, r| tx.send((id, r)).unwrap());
        // Yalnızca asılı kalmaya karşı (sonuç gelince hemen döner). 5 sn yük altında yetmiyordu:
        // nuxut-build.slice MemoryHigh kısmasında pam_exec fork+exec saniyelerce sürebiliyor.
        rx.recv_timeout(Duration::from_secs(120)).unwrap()
    }

    #[test]
    fn fake_sees_exact_bytes() {
        let fake = Arc::new(Fake {
            expected: "şifre".as_bytes(),
            seen: Default::default(),
        });
        let (id, r) = run(Arc::clone(&fake), "şifre");
        assert_eq!(id, 7);
        assert!(r.unwrap().outcome.is_success());
        let (_, r) = run(Arc::clone(&fake), "sifre");
        assert_eq!(r.unwrap().outcome, Outcome::Denied);
        assert_eq!(
            fake.seen.lock().unwrap().as_slice(),
            ["şifre".as_bytes(), b"sifre"]
        );
    }

    #[test]
    fn panic_is_none() {
        let (_, r) = run(Arc::new(Panics), "x");
        assert!(r.is_none());
    }

    #[test]
    fn real_pam_through_worker() {
        let dir = tempfile::tempdir().unwrap();
        // Yol çalışma zamanında çözülür: cargo yol değişince yeniden derlemez, derleme anındaki
        // `env!` silinmiş bir yolu gösterebilir (pam_exec ENOENT → Error(4); bkz.
        // nuxut-pam tests/pam_confdir.rs `fixture`).
        let root = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into());
        let script = root.join("../crates/nuxut-pam/tests/fixtures/check-password.sh");
        assert!(script.is_file(), "fikstür yok: {}", script.display());
        std::fs::write(
            dir.path().join("t"),
            format!(
                "auth required pam_exec.so quiet expose_authtok {}\n",
                script.display()
            ),
        )
        .unwrap();
        let service = Service::with_confdir("t", dir.path()).unwrap();
        let user = nuxut_pam::current_user().unwrap().name;
        let auth = Arc::new(PamAuthenticator::new(service, user));
        assert!(
            run(Arc::clone(&auth), "nuxut-test-sifresi")
                .1
                .unwrap()
                .outcome
                .is_success()
        );
        assert!(!run(auth, "yanlis").1.unwrap().outcome.is_success());
    }
}

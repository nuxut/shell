//! Canlı test için sahte greetd: `cargo run -p nuxut-greeter --example fake_greetd -- <soket>`.
//! Gerçek kimlik doğrulama yapmaz; kullanıcılar ve şifreler `tests/common/fake_greetd.rs`'te.
//! Olaylar stderr'e (şifre yazılmaz); `start_session` yanıtlandıktan sonra 0 ile çıkar.

#[path = "../tests/common/fake_greetd.rs"]
mod fake_greetd;

fn main() {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("kullanım: fake_greetd <soket yolu>");
        std::process::exit(2);
    };
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("soket açılamadı: {e}");
            std::process::exit(2);
        }
    };
    eprintln!("fake-greetd: dinleniyor");
    fake_greetd::serve(listener, |event| {
        eprintln!("fake-greetd: {event:?}");
        if matches!(event, fake_greetd::Logged::Start { .. }) {
            // Yanıt yazılsın diye kısa gecikmeyle çık.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                std::process::exit(0);
            });
        }
    });
}

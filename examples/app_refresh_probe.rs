//! Canlı yenilenmeyen uygulamalar için elle doğrulama aracı (yalnız yalıtılmış test oturumunda,
//! `dbus-run-session` içinde): `after <clients-json>` renk değişimi kararını uygular ve sorulacak
//! uygulamaları yazar; `restart <kimlik>` yeniden başlatır. docs/design/look-gtk.md.

use nuxut_shell::services::look::app_refresh::{self, App};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["after", clients] => {
            for app in app_refresh::after_colors_changed(Some(clients)) {
                println!("ask {}", app.id());
            }
        }
        ["restart", id] => match App::parse(id).map(app_refresh::restart) {
            Some(Ok(())) => println!("restarted"),
            Some(Err(e)) => println!("error {e}"),
            None => println!("unknown"),
        },
        _ => eprintln!("kullanım: app_refresh_probe after <json> | restart <kimlik>"),
    }
}

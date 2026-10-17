//! "Güncelleme bitti" bildirimi (2026-10-13; `docs/design/mascot-placement.md`).
//! Kural ve dosya `services::shell_state`'te; burada yalnızca açılıştaki çağrı ve bildirim
//! metni. install.sh sinyali yok: kabuk açılışta kendi sürümünü kayıtlı sürümle karşılaştırır.

use nuxut::Context;
use nuxut::i18n::t;

use super::Shell;
use crate::services::notifications::{InternalNotice, NoticeMood};
use crate::services::shell_state::{self, SHELL_VERSION};

/// Bildirim: başlık "Nuxut güncellendi", gövde yeni sürüm; kartta `Celebrate` maskotu
/// (`[appearance] mascot = false` iken maskotsuz normal kart, karar kartta).
pub fn update_notice(version: &str) -> InternalNotice {
    InternalNotice::popup(t!("shell-update-done-title"), t!("shell-update-done-body", version = version), Vec::new())
        .with_mascot(NoticeMood::Celebrate)
}

impl Shell {
    /// `start`'ta bir kez (bildirim sunucusundan sonra): sürüm arttıysa bildirimi bir kez
    /// gösterir. İlk kurulum, aynı/düşük sürüm ve bozuk dosya bildirimsizdir; yazamayan örnek
    /// (kilitsiz ya da kullanıcının evine yazamayan test örneği) hiç göstermez. `welcome`:
    /// Hoş geldin ekranda (`offer_welcome`) — bildirim bastırılır, `last_version` yine yazılır.
    pub(super) fn check_update_notice(&mut self, cx: &mut Context<Self>, welcome: bool) {
        if let Some(notice) = self.update_check(welcome) {
            self.notice(cx, notice);
        }
    }

    /// Bağlamsız karar ve dosya yazımı (testlenebilir); gösterilecek bildirim. Hoş geldin
    /// gösteriliyorsa (`welcome`) güncelleme bildirimi BASTIRILIR — ikisi asla birlikte çıkmaz
    /// (2026-10-13) — ama `last_version` yine yazılır: sonraki açılışta da çıkmaz.
    pub(super) fn update_check(&self, welcome: bool) -> Option<InternalNotice> {
        match shell_state::check(&self.apply_env, SHELL_VERSION) {
            Ok(c) if c.should_notify() && welcome => {
                tracing::info!(from = ?c.change, to = SHELL_VERSION, "kabuk güncellendi; Hoş geldin açık, bildirim bastırıldı");
                None
            }
            Ok(c) if c.should_notify() => {
                tracing::info!(from = ?c.change, to = SHELL_VERSION, "kabuk güncellendi; bildirim gösteriliyor");
                Some(update_notice(SHELL_VERSION))
            }
            Ok(c) => {
                tracing::debug!(change = ?c.change, written = c.written, "sürüm denetimi: bildirim yok");
                None
            }
            Err(e) => {
                tracing::debug!("sürüm durumu yazılmadı ({e}); güncelleme bildirimi yok");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ShellConfig;
    use crate::services::shell_state::{STATE_FILE, StateRead, read};

    fn shell_with_state(dir: &std::path::Path) -> Shell {
        let mut shell = Shell::new(ShellConfig::validated_default());
        shell.set_apply_env(nuxut_apply::Env::for_test(dir));
        shell
    }

    fn state_file(dir: &std::path::Path) -> std::path::PathBuf {
        dir.join("state/nuxut").join(STATE_FILE)
    }

    fn write_last(dir: &std::path::Path, version: &str) {
        let p = state_file(dir);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, format!("last_version = \"{version}\"\n")).unwrap();
    }

    #[test]
    fn notice_text_and_mascot() {
        crate::i18n::use_turkish_for_tests();
        let n = update_notice("1.2.3");
        assert_eq!(n.summary, "Nuxut güncellendi");
        assert!(n.body.contains("1.2.3"), "{}", n.body);
        assert!(n.popup);
        assert_eq!(n.mascot, Some(NoticeMood::Celebrate));
    }

    #[test]
    fn first_run_writes_but_does_not_notify() {
        let t = tempfile::tempdir().unwrap();
        let shell = shell_with_state(t.path());
        assert!(shell.update_check(false).is_none());
        assert!(matches!(read(&state_file(t.path())), StateRead::Present(r) if r.last_version.to_string() == SHELL_VERSION));
    }

    #[test]
    fn upgrade_notifies_once_then_never_again() {
        let t = tempfile::tempdir().unwrap();
        write_last(t.path(), "0.0.1-alpha");
        let mut shell = shell_with_state(t.path());
        let notice = shell.update_check(false).expect("artış bildirilir");
        assert_eq!(notice.mascot, Some(NoticeMood::Celebrate));
        shell.post_notice(notice);
        assert_eq!(shell.model.notifications.popups().count(), 1);
        assert!(shell.model.notifications.popups().all(|n| n.mascot == Some(NoticeMood::Celebrate)));
        // İkinci açılış.
        assert!(shell_with_state(t.path()).update_check(false).is_none());
    }

    #[test]
    fn same_downgrade_and_corrupt_do_not_notify() {
        let t = tempfile::tempdir().unwrap();
        write_last(t.path(), SHELL_VERSION);
        assert!(shell_with_state(t.path()).update_check(false).is_none(), "aynı");
        write_last(t.path(), "65535.0.0");
        assert!(shell_with_state(t.path()).update_check(false).is_none(), "düşüş");
        assert!(matches!(read(&state_file(t.path())), StateRead::Present(r) if r.last_version.to_string() == SHELL_VERSION));
        std::fs::write(state_file(t.path()), "\0\0bozuk").unwrap();
        assert!(shell_with_state(t.path()).update_check(false).is_none(), "bozuk: panik yok, bildirim yok");
        assert!(matches!(read(&state_file(t.path())), StateRead::Present(_)), "doğru değerle onarıldı");
    }

    /// Hoş geldin ekrandayken sürüm artışı bildirilmez; `last_version` yine yazılır, sonraki
    /// açılışta (Hoş geldin kapalı) da bildirim çıkmaz.
    #[test]
    fn welcome_suppresses_notice_but_records_version() {
        let t = tempfile::tempdir().unwrap();
        write_last(t.path(), "0.0.1");
        let shell = shell_with_state(t.path());
        assert!(shell.update_check(true).is_none(), "Hoş geldin varken bildirim yok");
        assert!(matches!(read(&state_file(t.path())), StateRead::Present(r) if r.last_version.to_string() == SHELL_VERSION));
        assert!(shell_with_state(t.path()).update_check(false).is_none(), "sürüm kaydedildi: tekrar çıkmaz");
    }

    /// Kilitsiz (yazamayan) örnek: dosya yok, bildirim yok.
    #[test]
    fn disabled_instance_never_notifies() {
        let t = tempfile::tempdir().unwrap();
        write_last(t.path(), "0.0.1");
        let mut shell = Shell::new(ShellConfig::validated_default());
        shell.set_apply_env(nuxut_apply::Env { disabled: true, ..nuxut_apply::Env::for_test(t.path()) });
        assert!(shell.update_check(false).is_none());
    }
}

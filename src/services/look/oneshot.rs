//! `nuxut-shell look apply` / `look revert`: birleşik görünümün tek seferlik, süreç içi uygulaması ve geri alınması.
//!
//! Çalışan shell'in görünüm thread'iyle aynı tur (`look::sync`): framework.toml (+ palet) okunur,
//! `[desktop] unified` açıksa plan `nuxut-apply` ile ayar başına tek işlem/snapshot yazılır, kapalı
//! ve etkinse Nuxut öncesine geri yüklenir. Çalışan bir shell gerekmez ve ona dokunulmaz (IPC yok).
//! Yazım ortamı `Env::from_process(instance)`: `NUXUT_SHELL_INSTANCE` tanımlıysa test örneği,
//! yalnızca kendi geçici XDG köklerine yazar ve canlı eylem göndermez. Aynı anda çalışan bir shell
//! ile çakışma `nuxut-apply` kilidiyle sıralanır; aynı girdi NoOp'tur.
//!
//! Hyprland yapılandırmasındaki imleç ortamı (`SyncResult::cursor_env`) burada YAZILMAZ: o,
//! çalışan shell'in Hyprland eşitlemesinin işidir; çıktıda yalnızca bilgi olarak verilir.
//!
//! Çıktı metinleri tanı metnidir (ADR-0010: CLI tanı çıktısı çevrilmez).

use serde::Serialize;

use super::backend::ApplyReport;
use super::writer::SystemLive;
use super::{LookState, sync};
use crate::services::hyprland::{HyprlandClient, HyprlandPaths};

/// `look apply` sonucu (JSON).
#[derive(Debug, Clone, Serialize)]
pub struct OneShot {
    /// Test örneği (geçici kökler; canlı eylem yok).
    pub test_instance: bool,
    pub unified: bool,
    /// `None`: kapalı ve hiç uygulanmamış (yapılacak iş yok).
    pub report: Option<ApplyReport>,
    /// Bilgi: çalışan shell'in Hyprland eşitlemesine giden imleç ortamı.
    pub cursor_env: Vec<(String, String)>,
    /// framework.toml uyarıları.
    pub warnings: Vec<String>,
}

impl OneShot {
    /// Başarılı mı (rapor yoksa yapılacak iş yoktu → başarılı).
    pub fn ok(&self) -> bool {
        self.report.as_ref().is_none_or(|r| r.ok)
    }
}

/// Bir tur çalıştırır. `instance`: `NUXUT_SHELL_INSTANCE` (boş/yok = kullanıcının oturumu).
pub fn apply_once(instance: Option<&str>) -> Result<OneShot, String> {
    let env = nuxut_apply::Env::from_process(instance.filter(|s| !s.is_empty()));
    let loaded = nuxut::Theme::load().map_err(|e| format!("framework.toml unreadable: {e}"))?;
    let state = LookState::read(&loaded.value)?;
    let hypr = if env.may_control_hyprland() {
        HyprlandPaths::from_env().ok().map(|p| HyprlandClient::new(p.socket1))
    } else {
        None
    };
    let mut sink = SystemLive::new(hypr, &env);
    let result = sync(&state, &env, &mut sink);
    Ok(OneShot {
        test_instance: env.test_instance,
        unified: state.framework.desktop.unified,
        report: result.report,
        cursor_env: result.cursor_env,
        warnings: loaded.warnings,
    })
}

/// `look revert` sonucu (JSON).
#[derive(Debug, Clone, Serialize)]
pub struct Revert {
    /// Test örneği (geçici kökler; canlı eylem yok).
    pub test_instance: bool,
    /// `None`: Nuxut'un görünümü diskte etkin değildi (geri alınacak bir şey yok).
    pub report: Option<ApplyReport>,
}

impl Revert {
    pub fn ok(&self) -> bool {
        self.report.as_ref().is_none_or(|r| r.ok)
    }
}

/// `nuxut-shell look revert`: birleşik görünümü `framework.toml`'dan bağımsız olarak Nuxut
/// öncesine geri alır — kapatma yolu (ADR-0004 §8-3, [`super::writer::deactivate`]): her `look.*`
/// ayarının sabitlenmiş işlemleri zorla geri yüklenir, ardından KDE sinyalleri, oturum ortamı
/// silme ve kalan imleçle `setcursor`. install.sh'in test dönemi taze kurulumu, snapshot deposunu
/// yedeğe taşımadan önce bunu çağırır. Çalışan shell'e dokunmaz; o açıksa ve birleşik görünüm
/// açıksa bir sonraki turda yeniden uygular (çağıran önce shell'i kapatır).
pub fn revert_once(instance: Option<&str>) -> Result<Revert, String> {
    let env = nuxut_apply::Env::from_process(instance.filter(|s| !s.is_empty()));
    let dirs = super::plan::Dirs::from_env().ok_or("HOME is not set")?;
    let hypr = if env.may_control_hyprland() {
        HyprlandPaths::from_env().ok().map(|p| HyprlandClient::new(p.socket1))
    } else {
        None
    };
    let mut sink = SystemLive::new(hypr, &env);
    let report = super::writer::deactivate(&env, &mut sink, &dirs);
    Ok(Revert { test_instance: env.test_instance, report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revert_ok_and_json() {
        let mut r = Revert { test_instance: true, report: None };
        assert!(r.ok());
        assert!(serde_json::to_string(&r).unwrap().contains("\"report\":null"));
        r.report = Some(ApplyReport { ok: false, ..ApplyReport::default() });
        assert!(!r.ok());
    }

    #[test]
    fn ok_without_report_and_with_failed_report() {
        let mut o = OneShot { test_instance: true, unified: false, report: None, cursor_env: Vec::new(), warnings: Vec::new() };
        assert!(o.ok());
        o.report = Some(ApplyReport { ok: false, ..ApplyReport::default() });
        assert!(!o.ok());
        o.report = Some(ApplyReport { ok: true, ..ApplyReport::default() });
        assert!(o.ok());
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"test_instance\":true") && json.contains("\"report\":{"), "{json}");
    }
}

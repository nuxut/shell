//! `msg power …`: güç durumu (tek satır JSON) ve pil şarj sınırı aç/kapa. Şarj sınırının yanıtı
//! UPower cevap verince gelir (`PowerEvent::ChargeLimitDone`); yanıt metni
//! `ok` ya da `error: <kod>: …` (kodlar `PowerError::code`; Ayarlar koda göre çevirir).

use super::Shell;
use crate::ipc::PowerCommand;
use crate::services::power::PowerError;

/// Aynı anda yanıt bekleyen en fazla istek; dolunca en eskisi yanıtsız düşer (istemci boş yanıt
/// alır). Servis bir isteği hiç yanıtlamasa da liste büyümez.
const MAX_PENDING: usize = 4;

impl Shell {
    /// `power status` / `power charge-limit status` (eşzamanlı; modeldeki son durum).
    pub(super) fn power_status_command(&self, c: PowerCommand) -> Result<String, String> {
        let state = &self.model.system.power;
        match c {
            PowerCommand::Status => Ok(state.status_json().to_string()),
            PowerCommand::ChargeLimitStatus => Ok(state.charge_limit_json().to_string()),
            PowerCommand::SetChargeLimit(_) => Err("power charge-limit on|off yalnızca nuxut-shell msg ile".into()),
        }
    }

    /// `power charge-limit on|off`: istek güç servisine gider, yanıt sonuç gelince.
    pub(super) fn charge_limit_command(&mut self, on: bool, reply: nuxut::ipc::Reply) {
        let Some(id) = self.system.as_ref().and_then(|s| s.set_charge_limit(on)) else {
            reply.send(&format!("error: {}: güç servisi çalışmıyor ya da meşgul", PowerError::Unavailable.code()));
            return;
        };
        tracing::info!(on, "şarj sınırı isteği");
        if self.charge_replies.len() >= MAX_PENDING {
            self.charge_replies.remove(0);
        }
        self.charge_replies.push((id, reply));
    }

    /// Güç servisinden sonuç geldi.
    pub(super) fn charge_limit_done(&mut self, id: u64, result: &Result<(), PowerError>) {
        let Some(i) = self.charge_replies.iter().position(|(r, _)| *r == id) else { return };
        let (_, reply) = self.charge_replies.swap_remove(i);
        reply.send(&charge_limit_reply(result));
    }
}

/// Yanıt metni (Türkçe açıklama yalnızca CLI kullanıcısı için; istemciler koda bakar).
fn charge_limit_reply(result: &Result<(), PowerError>) -> String {
    let e = match result {
        Ok(()) => return "ok".into(),
        Err(e) => e,
    };
    let text = match e {
        PowerError::Unavailable => "UPower çalışmıyor".to_owned(),
        PowerError::Unsupported => "bu pil şarj sınırını desteklemiyor".to_owned(),
        PowerError::Denied => "polkit izin vermedi (org.freedesktop.UPower.enable-charging-limit)".to_owned(),
        PowerError::Failed(name) => format!("UPower hatası ({name})"),
    };
    format!("error: {}: {text}", e.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_carry_machine_codes() {
        assert_eq!(charge_limit_reply(&Ok(())), "ok");
        assert!(charge_limit_reply(&Err(PowerError::Denied)).starts_with("error: denied: "));
        assert!(charge_limit_reply(&Err(PowerError::Unsupported)).starts_with("error: unsupported: "));
        assert!(charge_limit_reply(&Err(PowerError::Unavailable)).starts_with("error: unavailable: "));
        let failed = charge_limit_reply(&Err(PowerError::Failed("org.freedesktop.UPower.GeneralError".into())));
        assert!(failed.starts_with("error: failed: ") && failed.contains("GeneralError"));
    }
}

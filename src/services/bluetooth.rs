//! Bluetooth: BlueZ istemcisi `nuxut-bluez` crate'indedir (Ayarlar ile paylaşılır; UI'sız).
//! Burada yeniden dışa aktarma ve `msg bluetooth agent-status` yanıtı; hata türünün kullanıcı
//! metni `system.rs`'de (`network_error_text` / `bluetooth_error_text`).

pub use nuxut_bluez::*;

/// `msg bluetooth agent-status` yanıtı (tek satır JSON, çevrilmez — ADR-0010):
/// `{"registered":bool,"default":bool}`. Kaynak kabuğun model durumu: Bluetooth servisi
/// başlamadıysa ya da örnek ajansızsa durum varsayılandır (ikisi `false`); BlueZ gidince de.
pub fn agent_status_json(state: &BluetoothState) -> String {
    format!(r#"{{"registered":{},"default":{}}}"#, state.agent.registered, state.agent.default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_status_reply() {
        // Servis yok / ajansız örnek: model varsayılan.
        assert_eq!(agent_status_json(&BluetoothState::default()), r#"{"registered":false,"default":false}"#);
        let with_agent =
            BluetoothState { available: true, agent: AgentStatus { registered: true, default: true }, ..Default::default() };
        assert_eq!(agent_status_json(&with_agent), r#"{"registered":true,"default":true}"#);
        // Kayıtlı ama varsayılan olamadı (başka bir ajan varsayılan).
        let not_default =
            BluetoothState { available: true, agent: AgentStatus { registered: true, default: false }, ..Default::default() };
        assert_eq!(agent_status_json(&not_default), r#"{"registered":true,"default":false}"#);
    }
}

//! İkincil örnek kararı (saf, testli).
//!
//! Tek örnek kilidini başka bir `nuxut-lock` tutuyorsa yeni çağrı ancak o örnek GERÇEKTEN
//! kilitliyse (`status` → `locked`) hemen 0 ile çıkabilir. Aksi hâlde erken çıkmak fail-open
//! olur: birincil açılıştan sonra `Done`'da (LockedHint + roundtrip, IPC cevapsız) ya da
//! `Pending`'de reddedilecekken gelen boşta/uyku tetikleyicisi oturumu kilitsiz bırakırdı.
//!
//! - `pending` → kısa aralıklarla yeniden sorulur;
//! - `done` / boş / hata / bilinmeyen → birincilin çıkması beklenir (tek örnek kilidi yeniden
//!   denenir; boşalınca bu süreç kendisi kilitler);
//! - süre dolarsa kilit kurulamadı sayılır ([`crate::state::EXIT_NOT_LOCKED`]).

use std::time::Duration;

/// Yeniden deneme aralığı.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Toplam bekleme sınırı. Birincilin `Done` yolu (LockedHint en çok 1 sn + roundtrip) ve bir
/// cevapsız IPC isteği (framework `ipc::TIMEOUT` 2 sn) sığar; uyku inhibitor'ı yine
/// `InhibitDelayMaxUSec` ile sınırlıdır.
pub const WAIT_LIMIT: Duration = Duration::from_secs(8);

/// Birincil örneğin bildirdiği durum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    /// Oturum kilitli (`Locked` / `Unlocking`).
    Locked,
    /// Kilit istendi, compositor henüz onaylamadı.
    Pending,
    /// Bitiyor, cevap yok ya da anlaşılmadı: kilit var sayılmaz.
    NotLocked,
}

/// `status` cevabını yorumlar; IPC hatası `None`.
pub fn parse_status(reply: Option<&str>) -> PeerState {
    match reply.map(str::trim) {
        Some("locked") => PeerState::Locked,
        Some("pending") => PeerState::Pending,
        _ => PeerState::NotLocked,
    }
}

/// İkincil örneğin sıradaki adımı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Birincil kilitli: ready yazılır, 0 ile çıkılır.
    AlreadyLocked,
    /// Bekle, sonra tek örnek kilidini yeniden dene (boşaldıysa kendin kilitle).
    Retry(Duration),
    /// Süre doldu ve kilit görülmedi: kilit kurulamadı.
    GiveUp,
}

/// `waited`: ilk denemeden bu yana geçen süre.
pub fn decide(peer: PeerState, waited: Duration) -> Step {
    match peer {
        PeerState::Locked => Step::AlreadyLocked,
        PeerState::Pending | PeerState::NotLocked if waited < WAIT_LIMIT => {
            Step::Retry(POLL_INTERVAL)
        }
        PeerState::Pending | PeerState::NotLocked => Step::GiveUp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_locked_reply_counts_as_locked() {
        assert_eq!(parse_status(Some("locked\n")), PeerState::Locked);
        assert_eq!(parse_status(Some(" pending ")), PeerState::Pending);
        for r in [Some("done"), Some(""), Some("LOCKED"), Some("locked?"), None] {
            assert_eq!(parse_status(r), PeerState::NotLocked, "{r:?}");
        }
    }

    #[test]
    fn early_success_only_when_locked() {
        assert_eq!(decide(PeerState::Locked, Duration::ZERO), Step::AlreadyLocked);
        assert_eq!(decide(PeerState::Locked, WAIT_LIMIT * 2), Step::AlreadyLocked);
        assert_eq!(decide(PeerState::Pending, Duration::ZERO), Step::Retry(POLL_INTERVAL));
        assert_eq!(decide(PeerState::NotLocked, Duration::ZERO), Step::Retry(POLL_INTERVAL));
    }

    #[test]
    fn waiting_is_bounded_and_fails_closed() {
        for peer in [PeerState::Pending, PeerState::NotLocked] {
            assert_eq!(decide(peer, WAIT_LIMIT), Step::GiveUp);
            assert_eq!(decide(peer, WAIT_LIMIT + POLL_INTERVAL), Step::GiveUp);
        }
    }
}

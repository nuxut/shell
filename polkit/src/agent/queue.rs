//! İstek kuyruğu: saf durum makinesi (D-Bus, süreç, zaman bilmez; testli).
//!
//! - Aynı anda tek diyalog. Diğer istekler sırayla bekler (en çok [`MAX_WAITING`]).
//! - Etkin istek iptal edilince polkitd'ye hemen `Cancelled` dönülür, diyalog sonlandırılır;
//!   yuva diyalog süreci gerçekten çıkana dek dolu kalır (ekranda iki diyalog olmaz).
//! - Diyalog sonucu her istek için en çok bir yanıt üretir; geç gelen sonuç yok sayılır.
//! - polkitd giderse (`NameOwnerChanged`) tüm istekler `Cancelled` ile kapanır.

use std::collections::VecDeque;

use crate::request::DialogOutcome;

/// Kuyrukta bekleyebilecek istek sayısı (etkin hariç).
pub const MAX_WAITING: usize = 16;

pub type RequestId = u64;

/// polkitd'ye dönülecek yanıt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// `BeginAuthentication` normal döner; yetki kararı polkitd'nin `is_authenticated`'ı.
    Done,
    /// `org.freedesktop.PolicyKit1.Error.Cancelled`.
    Cancelled,
    /// `org.freedesktop.PolicyKit1.Error.Failed`.
    Failed,
}

impl From<DialogOutcome> for Reply {
    fn from(o: DialogOutcome) -> Self {
        match o {
            DialogOutcome::Success => Self::Done,
            DialogOutcome::Cancelled => Self::Cancelled,
            DialogOutcome::Failed => Self::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bu istek için diyalog sürecini başlat (ve süre sınırını kur).
    Spawn(RequestId),
    /// Diyalog sürecini sonlandır (SIGTERM; süre içinde çıkmazsa SIGKILL).
    Kill(RequestId),
    Reply(RequestId, Reply),
}

/// Reddedilen yeni istek.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    DuplicateCookie,
    QueueFull,
}

#[derive(Debug)]
struct Active {
    id: RequestId,
    cookie: String,
    /// polkitd'ye yanıt verildi (iptal); süreç çıkışı bekleniyor.
    replied: bool,
}

#[derive(Debug, Default)]
pub struct Queue {
    active: Option<Active>,
    waiting: VecDeque<(RequestId, String)>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> Option<RequestId> {
        self.active.as_ref().map(|a| a.id)
    }

    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    fn live_cookie(&self, cookie: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| !a.replied && a.cookie == cookie)
            || self.waiting.iter().any(|(_, c)| c == cookie)
    }

    pub fn begin(&mut self, id: RequestId, cookie: &str) -> Result<Vec<Action>, Reject> {
        if self.live_cookie(cookie) {
            return Err(Reject::DuplicateCookie);
        }
        if self.active.is_none() {
            self.active = Some(Active {
                id,
                cookie: cookie.to_owned(),
                replied: false,
            });
            return Ok(vec![Action::Spawn(id)]);
        }
        if self.waiting.len() >= MAX_WAITING {
            return Err(Reject::QueueFull);
        }
        self.waiting.push_back((id, cookie.to_owned()));
        Ok(Vec::new())
    }

    /// `CancelAuthentication(cookie)`. `None`: bu çerezle bekleyen istek yok.
    pub fn cancel(&mut self, cookie: &str) -> Option<Vec<Action>> {
        if let Some(pos) = self.waiting.iter().position(|(_, c)| c == cookie) {
            let (id, _) = self.waiting.remove(pos)?;
            return Some(vec![Action::Reply(id, Reply::Cancelled)]);
        }
        let a = self.active.as_mut()?;
        if a.replied || a.cookie != cookie {
            return None;
        }
        a.replied = true;
        Some(vec![Action::Reply(a.id, Reply::Cancelled), Action::Kill(a.id)])
    }

    /// Süre sınırı doldu (diyalog takıldı): iptal gibi.
    pub fn deadline(&mut self, id: RequestId) -> Vec<Action> {
        match self.active.as_mut() {
            Some(a) if a.id == id && !a.replied => {
                a.replied = true;
                vec![Action::Reply(id, Reply::Cancelled), Action::Kill(id)]
            }
            _ => Vec::new(),
        }
    }

    /// Diyalog süreci çıktı (sonuç satırıyla ya da satırsız → `Failed`).
    pub fn dialog_exited(&mut self, id: RequestId, outcome: DialogOutcome) -> Vec<Action> {
        let Some(a) = self.active.as_ref().filter(|a| a.id == id) else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if !a.replied {
            actions.push(Action::Reply(id, outcome.into()));
        }
        self.active = None;
        actions.extend(self.next());
        actions
    }

    fn next(&mut self) -> Option<Action> {
        let (id, cookie) = self.waiting.pop_front()?;
        self.active = Some(Active {
            id,
            cookie,
            replied: false,
        });
        Some(Action::Spawn(id))
    }

    /// Diyalog başlatılamadı: `Failed` ve sıradaki.
    pub fn spawn_failed(&mut self, id: RequestId) -> Vec<Action> {
        self.dialog_exited(id, DialogOutcome::Failed)
    }

    /// polkitd gitti/değişti: herkese `Cancelled`, etkin diyalog sonlandırılır. Etkin yuva
    /// süreç çıkana dek dolu kalır.
    pub fn authority_gone(&mut self) -> Vec<Action> {
        let mut actions: Vec<Action> = self
            .waiting
            .drain(..)
            .map(|(id, _)| Action::Reply(id, Reply::Cancelled))
            .collect();
        if let Some(a) = self.active.as_mut()
            && !a.replied
        {
            a.replied = true;
            actions.push(Action::Reply(a.id, Reply::Cancelled));
            actions.push(Action::Kill(a.id));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_dialog_and_fifo() {
        let mut q = Queue::new();
        assert_eq!(q.begin(1, "a"), Ok(vec![Action::Spawn(1)]));
        assert_eq!(q.begin(2, "b"), Ok(vec![]));
        assert_eq!(q.begin(3, "c"), Ok(vec![]));
        assert_eq!(q.begin(4, "b"), Err(Reject::DuplicateCookie));
        assert_eq!(q.begin(5, "a"), Err(Reject::DuplicateCookie));
        assert_eq!(
            q.dialog_exited(1, DialogOutcome::Success),
            [Action::Reply(1, Reply::Done), Action::Spawn(2)]
        );
        assert_eq!(
            q.dialog_exited(2, DialogOutcome::Failed),
            [Action::Reply(2, Reply::Failed), Action::Spawn(3)]
        );
        assert_eq!(q.dialog_exited(3, DialogOutcome::Cancelled), [Action::Reply(3, Reply::Cancelled)]);
        assert_eq!(q.active(), None);
        // Geç gelen sonuç yok sayılır.
        assert!(q.dialog_exited(3, DialogOutcome::Success).is_empty());
    }

    #[test]
    fn cancel_active_waits_for_exit() {
        let mut q = Queue::new();
        q.begin(1, "a").unwrap();
        q.begin(2, "b").unwrap();
        assert_eq!(
            q.cancel("a"),
            Some(vec![Action::Reply(1, Reply::Cancelled), Action::Kill(1)])
        );
        assert_eq!(q.cancel("a"), None, "ikinci iptal yanıtsız");
        // Yuva dolu: yeni diyalog süreç çıkana dek açılmaz; aynı çerez yeniden kullanılabilir.
        assert_eq!(q.begin(3, "a"), Ok(vec![]));
        // Öldürülen diyaloğun "success" satırı ikinci yanıt üretmez.
        assert_eq!(q.dialog_exited(1, DialogOutcome::Success), [Action::Spawn(2)]);
    }

    #[test]
    fn cancel_waiting_and_unknown() {
        let mut q = Queue::new();
        q.begin(1, "a").unwrap();
        q.begin(2, "b").unwrap();
        assert_eq!(q.cancel("b"), Some(vec![Action::Reply(2, Reply::Cancelled)]));
        assert_eq!(q.cancel("zz"), None);
        assert_eq!(q.dialog_exited(1, DialogOutcome::Success), [Action::Reply(1, Reply::Done)]);
    }

    #[test]
    fn queue_limit() {
        let mut q = Queue::new();
        q.begin(0, "x").unwrap();
        for i in 0..MAX_WAITING as u64 {
            q.begin(i + 1, &format!("c{i}")).unwrap();
        }
        assert_eq!(q.begin(99, "over"), Err(Reject::QueueFull));
        assert_eq!(q.waiting(), MAX_WAITING);
    }

    #[test]
    fn deadline_and_authority_gone() {
        let mut q = Queue::new();
        q.begin(1, "a").unwrap();
        q.begin(2, "b").unwrap();
        assert!(q.deadline(2).is_empty(), "bekleyen isteğin süresi yok");
        assert_eq!(q.deadline(1), [Action::Reply(1, Reply::Cancelled), Action::Kill(1)]);
        assert!(q.deadline(1).is_empty());
        let mut q = Queue::new();
        q.begin(1, "a").unwrap();
        q.begin(2, "b").unwrap();
        assert_eq!(
            q.authority_gone(),
            [
                Action::Reply(2, Reply::Cancelled),
                Action::Reply(1, Reply::Cancelled),
                Action::Kill(1)
            ]
        );
        assert_eq!(q.dialog_exited(1, DialogOutcome::Failed), []);
        assert_eq!(q.active(), None);
    }

    #[test]
    fn spawn_failure_moves_on() {
        let mut q = Queue::new();
        q.begin(1, "a").unwrap();
        q.begin(2, "b").unwrap();
        assert_eq!(q.spawn_failed(1), [Action::Reply(1, Reply::Failed), Action::Spawn(2)]);
    }
}

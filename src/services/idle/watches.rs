//! Compositor boşta izlemeleri (ext-idle-notify-v1) ile aşamalar arasındaki eşleme.
//!
//! [`super::Effect::Watch`] "bu aşamalar, bu sürelerle, sayaçlar şimdiden" der. Defter bunu
//! izleme kurma/yeniden kaydetme/bırakma çağrılarına çevirir ve gelen olayın kimliğini aşamaya
//! geri çevirir. Arka uç ([`IdleWatcher`]) uygulamada framework `Context`'i, testte sahtedir;
//! defter Wayland'den bağımsızdır.
//!
//! Sözleşme:
//! - Kalan aşama yeniden kaydedilir (süre aynı olsa da): protokolde sayaç sıfırlama yok,
//!   framework `idle_set_timeout` nesneyi yeniden yaratır — `Watch`'ın "sayaçlar şimdiden"
//!   anlamı budur. Kimlik değişmez.
//! - Kalkan aşamanın izlemesi bırakılır; yeni aşama için izleme kurulur.
//! - Arka uç kimliği tanımıyorsa (`set_timeout` → `false`) yeni izleme kurulur.
//! - Compositor desteklemiyorsa sessiz yedek yok: defter boş kalır, durum `Unsupported`.

use std::time::Duration;

use super::Stage;

/// İzleme arka ucu (uygulamada `nuxut::Context`).
pub trait IdleWatcher {
    type Id: Copy + Eq + std::fmt::Debug;
    type Error: std::fmt::Display;
    /// Wayland idle inhibitor'larına saygılı izleme kurar.
    fn watch(&mut self, timeout: Duration) -> Result<Self::Id, Self::Error>;
    /// Süreyi değiştirir ve sayacı baştan başlatır; bilinmeyen kimlikte `false`.
    fn set_timeout(&mut self, id: Self::Id, timeout: Duration) -> bool;
    fn unwatch(&mut self, id: Self::Id);
}

/// Boşta bildiriminin kaynağı (`msg idle status` → `source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchSource {
    /// Henüz hiçbir aşama istenmedi ya da izleme kurulmadı (destek bilinmiyor).
    Idle,
    /// Compositor izlemeleri kuruldu.
    Compositor,
    /// Compositor ext-idle-notify-v1 sunmuyor; aşamalar çalışmaz.
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct Watches<Id> {
    active: Vec<(Stage, Id, Duration)>,
    source: WatchSource,
}

impl<Id> Default for Watches<Id> {
    fn default() -> Self {
        Self { active: Vec::new(), source: WatchSource::Idle }
    }
}

impl<Id: Copy + Eq + std::fmt::Debug> Watches<Id> {
    /// İzlenen aşamaları `want` ile eşler; tüm sayaçlar şimdiden başlar.
    pub fn sync<W: IdleWatcher<Id = Id>>(&mut self, backend: &mut W, want: &[(Stage, Duration)]) {
        // Kalkan aşamalar.
        self.active.retain(|&(stage, id, _)| {
            let keep = want.iter().any(|&(s, _)| s == stage);
            if !keep {
                backend.unwatch(id);
            }
            keep
        });
        for &(stage, timeout) in want {
            if let Some(entry) = self.active.iter_mut().find(|(s, _, _)| *s == stage) {
                if backend.set_timeout(entry.1, timeout) {
                    entry.2 = timeout;
                    continue;
                }
                // Arka uç kimliği unuttu (olmamalı): yeniden kur.
                self.active.retain(|(s, _, _)| *s != stage);
            }
            match backend.watch(timeout) {
                Ok(id) => {
                    self.active.push((stage, id, timeout));
                    self.source = WatchSource::Compositor;
                }
                Err(e) => {
                    // Sessiz yedek yok: tüm izlemeler bırakılır, durumda görünür.
                    if self.source != WatchSource::Unsupported {
                        tracing::warn!("boşta: compositor izlemesi kurulamadı ({e}); boşta aşamaları çalışmayacak");
                    }
                    for (_, id, _) in self.active.drain(..) {
                        backend.unwatch(id);
                    }
                    self.source = WatchSource::Unsupported;
                    return;
                }
            }
        }
    }

    /// Olayın kimliğinden aşama (bırakılmış ya da yabancı kimlikte `None`).
    pub fn stage(&self, id: Id) -> Option<Stage> {
        self.active.iter().find(|(_, i, _)| *i == id).map(|(s, _, _)| *s)
    }

    pub fn source(&self) -> WatchSource {
        self.source
    }

    /// İzlenen aşamalar ve süreleri (kurulma sırasıyla).
    pub fn stages(&self) -> impl Iterator<Item = (Stage, Duration)> + '_ {
        self.active.iter().map(|&(s, _, d)| (s, d))
    }

    pub fn len(&self) -> usize {
        self.active.len()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        Watch(u32, Duration),
        Retime(u32, Duration),
        Unwatch(u32),
    }

    #[derive(Default)]
    struct Fake {
        next: u32,
        live: Vec<u32>,
        calls: Vec<Call>,
        unsupported: bool,
    }

    impl IdleWatcher for Fake {
        type Id = u32;
        type Error = &'static str;
        fn watch(&mut self, t: Duration) -> Result<u32, &'static str> {
            if self.unsupported {
                return Err("yok");
            }
            self.next += 1;
            self.live.push(self.next);
            self.calls.push(Call::Watch(self.next, t));
            Ok(self.next)
        }
        fn set_timeout(&mut self, id: u32, t: Duration) -> bool {
            self.calls.push(Call::Retime(id, t));
            self.live.contains(&id)
        }
        fn unwatch(&mut self, id: u32) {
            self.live.retain(|&i| i != id);
            self.calls.push(Call::Unwatch(id));
        }
    }

    #[test]
    fn add_retime_remove() {
        let mut fake = Fake::default();
        let mut w = Watches::default();
        assert_eq!(w.source(), WatchSource::Idle);

        w.sync(&mut fake, &[(Stage::Dim, 30 * S), (Stage::Lock, 600 * S)]);
        assert_eq!(fake.calls, [Call::Watch(1, 30 * S), Call::Watch(2, 600 * S)]);
        assert_eq!(w.stage(1), Some(Stage::Dim));
        assert_eq!(w.stage(2), Some(Stage::Lock));
        assert_eq!(w.source(), WatchSource::Compositor);

        // Süre değişti, kilit kalktı, ekran kapatma eklendi.
        fake.calls.clear();
        w.sync(&mut fake, &[(Stage::Dim, 60 * S), (Stage::ScreenOff, 120 * S)]);
        assert_eq!(fake.calls, [Call::Unwatch(2), Call::Retime(1, 60 * S), Call::Watch(3, 120 * S)]);
        assert_eq!(w.stage(1), Some(Stage::Dim), "kimlik korunur");
        assert_eq!(w.stage(2), None, "bırakılan kimliğin geç olayı yok sayılır");
        assert_eq!(w.stage(3), Some(Stage::ScreenOff));
        assert_eq!(w.stages().collect::<Vec<_>>(), [(Stage::Dim, 60 * S), (Stage::ScreenOff, 120 * S)]);

        // Hepsi kapandı.
        fake.calls.clear();
        w.sync(&mut fake, &[]);
        assert_eq!(fake.calls, [Call::Unwatch(1), Call::Unwatch(3)]);
        assert!(w.is_empty() && fake.live.is_empty());
    }

    #[test]
    fn same_stages_restart_counters() {
        // Engel bitti / uyanış: aynı aşamalar yeniden istenir, sayaç sıfırlanmalı.
        let mut fake = Fake::default();
        let mut w = Watches::default();
        w.sync(&mut fake, &[(Stage::Dim, 30 * S)]);
        fake.calls.clear();
        w.sync(&mut fake, &[(Stage::Dim, 30 * S)]);
        assert_eq!(fake.calls, [Call::Retime(1, 30 * S)]);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn forgotten_id_is_rewatched() {
        let mut fake = Fake::default();
        let mut w = Watches::default();
        w.sync(&mut fake, &[(Stage::Dim, 30 * S)]);
        fake.live.clear();
        fake.calls.clear();
        w.sync(&mut fake, &[(Stage::Dim, 30 * S)]);
        assert_eq!(fake.calls, [Call::Retime(1, 30 * S), Call::Watch(2, 30 * S)]);
        assert_eq!(w.stage(1), None);
        assert_eq!(w.stage(2), Some(Stage::Dim));
    }

    #[test]
    fn unsupported_is_visible_not_silent() {
        let mut fake = Fake { unsupported: true, ..Fake::default() };
        let mut w = Watches::default();
        w.sync(&mut fake, &[]);
        assert_eq!(w.source(), WatchSource::Idle, "aşama yokken destek sorulmaz");
        w.sync(&mut fake, &[(Stage::Dim, 30 * S)]);
        assert_eq!(w.source(), WatchSource::Unsupported);
        assert!(w.is_empty());
    }

    #[test]
    fn failure_midway_releases_everything() {
        // Global iki çağrı arasında geri çekildi: yarım kurulum kalmaz.
        struct Flaky(Fake, u32);
        impl IdleWatcher for Flaky {
            type Id = u32;
            type Error = &'static str;
            fn watch(&mut self, t: Duration) -> Result<u32, &'static str> {
                if self.1 == 0 {
                    return Err("gitti");
                }
                self.1 -= 1;
                self.0.watch(t)
            }
            fn set_timeout(&mut self, id: u32, t: Duration) -> bool {
                self.0.set_timeout(id, t)
            }
            fn unwatch(&mut self, id: u32) {
                self.0.unwatch(id)
            }
        }
        let mut b = Flaky(Fake::default(), 1);
        let mut w = Watches::default();
        w.sync(&mut b, &[(Stage::Dim, 30 * S), (Stage::Suspend, 900 * S)]);
        assert_eq!(w.source(), WatchSource::Unsupported);
        assert!(w.is_empty() && b.0.live.is_empty());
    }
}

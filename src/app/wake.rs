//! Ekranların zamanlı uyanma isteği (`Ui::wake_at`: ipucu gecikmesi, Slider değer balonunun
//! sönme anı, CurveEditor önizleme beklemesi): beklerken kare yok. TEK bir timer tüm
//! ekranların en erken anına kurulur; çaldığında anı gelen her ekranda kare istenir ve timer
//! kalanların en erkenine yeniden kurulur. Bir monitörün isteği diğerininkini iptal etmez.

use std::time::Instant;

use nuxut::{Context, TimerId};

use super::Shell;

impl Shell {
    /// Tek uyanma timer'ını ekranların en erken anına uydurur (aynı ansa dokunmaz, yoksa iptal).
    /// Her kare ve işaretçi olayının sonunda çağrılır — erken dönüşler dahil, ekranın uyanma anı
    /// değişmiş olabilecek her yoldan sonra.
    pub(super) fn rearm_wake(&mut self, cx: &mut Context<Self>) {
        let at = self.screens.iter().filter_map(|s| s.wake()).min();
        if self.wake_timer.map(|(_, a)| a) == at {
            return;
        }
        if let Some((t, _)) = self.wake_timer.take() {
            cx.cancel_timer(t);
        }
        if let Some(at) = at {
            let t = cx.set_timer(at.saturating_duration_since(Instant::now()));
            self.wake_timer = Some((t, at));
        }
    }

    /// Uyanma timer'ıysa anı gelen ekranlarda kare ister, timer'ı yeniden kurar ve `true` döner.
    pub(super) fn wake_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.wake_timer.is_none_or(|(t, _)| t != id) {
            return false;
        }
        self.wake_timer = None;
        let now = Instant::now();
        for s in &mut self.screens {
            if s.take_due_wake(now) {
                cx.request_frame(s.main);
            }
        }
        self.rearm_wake(cx);
        true
    }
}

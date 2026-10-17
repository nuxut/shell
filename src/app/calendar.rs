//! Takvim istemcisi bağlantısı: görünür takvimlerin aralık sorgusu, gelen olaylar.
//!
//! Servis tek aralık sorgular ([`CalendarService::set_range`]); takvim ise birden çok yerde
//! görünebilir (her ekranın saat popout'u, açık/sabit panellerdeki takvim kartları) ve her biri
//! farklı ay gösterebilir. Görünür tüm takvimlerin aralıkları birleştirilip tek sorgu yapılır
//! ([`query_range`]; çok genişse son ay değiştirenin aralığı); her görünüm kendi günlerini aynı
//! `model.calendar`'dan süzer (`docs/design/calendar-card.md`).

use nuxut::Context;

use super::{Shell, ShellMessage};
use crate::calendar_view::query_range;
use crate::services::calendar::{CalendarEvent, CalendarOptions, CalendarService};

/// Sorgu durumu: son gönderilen birleşik aralık ve son ay değiştiren görünümün aralığı (çok
/// geniş birleşimde öncelik). Saf durum; servisi `Shell` çağırır.
#[derive(Debug, Default)]
pub(super) struct CalendarQuery {
    sent: Option<(i64, i64)>,
    focus: Option<(i64, i64)>,
}

impl CalendarQuery {
    /// Bir görünüm ay değiştirdi: aralığı odak olur.
    pub(super) fn focus(&mut self, range: (i64, i64)) {
        self.focus = Some(range);
    }

    /// Görünür aralıklardan gönderilecek sorgu. Görünür takvim yoksa ya da sorgu son
    /// gönderilenle aynıysa `None` (`force`: aynı da olsa gönder — yaprak açılınca tazele).
    pub(super) fn next(&mut self, visible: &[(i64, i64)], force: bool) -> Option<(i64, i64)> {
        let q = query_range(visible, self.focus)?;
        if !force && self.sent == Some(q) {
            return None;
        }
        self.sent = Some(q);
        Some(q)
    }
}

impl Shell {
    pub(super) fn start_calendar(&mut self, cx: &mut Context<Self>) {
        let sender = cx.sender();
        match CalendarService::spawn(CalendarOptions::default(), move |e| {
            let _ = sender.send(ShellMessage::Calendar(e));
        }) {
            Ok(service) => self.calendar = Some(service),
            Err(e) => tracing::warn!("takvim istemcisi başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_calendar(&mut self, cx: &mut Context<Self>, e: CalendarEvent) {
        if self.model.apply_calendar(e) {
            self.model_changed(cx);
        }
    }

    /// Bir takvim görünümü (popout ya da kart) ay değiştirdi.
    pub(super) fn calendar_range_changed(&mut self, from: i64, to: i64) {
        self.calendar_query.focus((from, to));
        self.refresh_calendar_range(false);
    }

    /// Görünür takvimler değişmiş olabilir (yaprak açıldı/kapandı, config, dil): birleşik
    /// aralığı sorgular. `force`: aynı aralık da yeniden sorgulanır.
    pub(super) fn refresh_calendar_range(&mut self, force: bool) {
        // Servis yokken (henüz başlamadı ya da başlatılamadı) sorgu "gönderildi" sayılmasın.
        if self.calendar.is_none() {
            return;
        }
        let mut visible: Vec<(i64, i64)> = self.screens.iter().flat_map(|s| s.calendar_ranges()).collect();
        // Bar'da sıradaki etkinlik varsa bugün + yarın da sorgulanır (günde bir değişir; saat
        // vuruşu yeniler). Ay ızgarası çok uzaktaysa birleşim sınırı aşar, gösterilen ay öncelikli.
        let bar = &self.model.config.bar;
        if crate::config::BarSection::ALL.into_iter().any(|s| bar.widgets_in(s).contains(&crate::widgets::WidgetKind::NextEvent)) {
            visible.push(crate::widgets::next_event_range(chrono::Local::now()));
        }
        if let Some((from, to)) = self.calendar_query.next(&visible, force)
            && let Some(c) = &self.calendar
        {
            c.set_range(from, to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar_view::{MAX_QUERY_SPAN, grid_range};

    #[test]
    fn popout_and_card_on_different_months_share_one_query() {
        let oct = grid_range(2026, 10, 0, 0);
        let nov = grid_range(2026, 11, 0, 0);
        let mut q = CalendarQuery::default();
        // Yalnızca sabit paneldeki kart (Ekim).
        assert_eq!(q.next(&[oct], false), Some(oct));
        assert_eq!(q.next(&[oct], false), None, "değişmedi: yeniden sorgu yok");
        assert_eq!(q.next(&[oct], true), Some(oct), "yaprak açıldı: tazele");
        // Popout açıldı ve Kasım'a geçti; kart Ekim'de kaldı: ikisi de kapsanır.
        q.focus(nov);
        assert_eq!(q.next(&[nov, oct], false), Some((oct.0, nov.1)));
        // Popout kapandı: yalnızca kartın ayı.
        assert_eq!(q.next(&[oct], false), Some(oct));
        // Hiç görünür takvim yok: son sorgu korunur, istek yok.
        assert_eq!(q.next(&[], true), None);
        assert_eq!(q.next(&[oct], false), None);
    }

    #[test]
    fn far_apart_views_query_the_focused_one() {
        let oct = grid_range(2026, 10, 0, 0);
        let far = grid_range(2028, 6, 0, 0);
        assert!(far.1 - oct.0 > MAX_QUERY_SPAN);
        let mut q = CalendarQuery::default();
        q.focus(far);
        assert_eq!(q.next(&[oct, far], false), Some(far));
        q.focus(oct);
        assert_eq!(q.next(&[oct, far], false), Some(oct));
    }
}

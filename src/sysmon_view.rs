//! Sistem izleyicisi popout'unun içeriği (`PanelId::Sysmon`): iki sütun — solda işlemci
//! (çekirdek başına mini çubuklar, yük, frekans), bellek + takas, GPU'lar; sağda sıcaklıklar,
//! ağ hızı, disk (bağlama noktaları + G/Ç) — altta tam genişlikte en çok CPU kullanan süreçler.
//!
//! Veri `model.sysmon`'dan (`services::sysmon`); panel açıkken servis ayrıntı kipindedir.
//! Olmayan ölçüm bölümü gizlidir (ör. VM'de sıcaklık/GPU yok). Çubuklar framework `Progress`,
//! kullanım ≥ %90'da framework `Threshold` rengine (varsayılan `error` rolü) geçer; işlemci,
//! bellek ve GPU başlığının altında servisin tuttuğu geçmiş `Sparkline` ile (yeni örnek sağdan
//! kayarak girer; ardışık olmayan geçmiş animasyonsuz yüklenir). Çubuklarda
//! (değer yayla akar); bölümler aynı kaldıkça bileşenler korunur, yalnızca metin/değer
//! güncellenir. Yükseklik içerikten: değişirse `Screen` yaprağı yeni hedefe akıtır.

use std::sync::Arc;
use std::time::Instant;

use nuxut::i18n::t;
use nuxut::render::TextAlign;
use nuxut::ui::style::THRESHOLD_HYSTERESIS;
use nuxut::ui::{Icon, IconName, IconView, Label, Progress, SparkScale, Sparkline, TextRole, Threshold, Ui, text_style};
use nuxut::{Point, Rect, TextSystem, Theme};

use crate::services::sysmon::history::{HISTORY_LEN, Series};
use crate::services::sysmon::{ALERT_PERCENT, History, Metric, Snapshot, Usage, format};

/// Popout genişliği (iki sütun; yükseklik içerikten).
pub const SYSMON_WIDTH: f32 = 600.0;
/// Gösterilen en çok çekirdek çubuğu (fazlası notta).
const MAX_CORE_BARS: usize = 32;
/// Bölüm başına en çok satır (sıcaklık, bağlama noktası).
const MAX_ROWS: usize = 6;
/// Çekirdek ızgarasının sütun sayısı.
const CORE_COLUMNS: usize = 4;

/// Satır: solda ad, sağda değer; isteğe bağlı çubuk (bağlama noktası) ve ek sütun (süreç belleği).
#[derive(Debug)]
struct Row {
    left: Label,
    right: Label,
    extra: Option<Label>,
    bar: Option<Progress>,
}

/// Geçmiş grafiği ve son yüklenen geçmişin `seq`'i.
#[derive(Debug)]
struct Spark {
    metric: Metric,
    line: Sparkline,
    seq: Option<u64>,
}

impl Spark {
    fn new(theme: &Theme, metric: Metric) -> Self {
        let line = Sparkline::new(theme, HISTORY_LEN).with_scale(PERCENT_SCALE).with_threshold(spark_threshold());
        Self { metric, line, seq: None }
    }

    /// Servis geçmişine eşitler: bir sonraki ölçümse yalnızca yeni örnek itilir (kayar), aksi
    /// hâlde (ilk kurulum, atlanan ölçüm, servis yeniden başladı) tümü animasyonsuz yüklenir.
    fn sync(&mut self, history: &History, now: Instant) {
        let series: &Series = history.series(self.metric);
        let seq = history.seq();
        match (self.seq, series.latest()) {
            (Some(old), _) if old == seq => {}
            (Some(old), Some(v)) if seq == old.wrapping_add(1) && self.line.history().len() + 1 >= series.len() => {
                self.line.push(v, now)
            }
            _ => self.line.set_samples(series.iter()),
        }
        self.seq = Some(seq);
    }
}

/// Yüzde geçmişinin dikey ekseni (sabit: 0..100; boşta çizgi tabanda kalır).
const PERCENT_SCALE: SparkScale = SparkScale::Fixed { min: 0.0, max: 100.0 };

/// Kullanım çubuklarının eşiği (0..=1; renk framework varsayılanı `error` rolü).
fn usage_threshold() -> Threshold {
    Threshold::new(ALERT_PERCENT / 100.0)
}

/// Geçmiş grafiğinin eşiği (% biriminde; geri dönüş payı çubuklarla aynı oranda).
fn spark_threshold() -> Threshold {
    Threshold::new(ALERT_PERCENT).with_hysteresis(THRESHOLD_HYSTERESIS * 100.0)
}

#[derive(Debug)]
struct Section {
    icon: IconView,
    title: Label,
    value: Label,
    spark: Option<Spark>,
    bar: Option<Progress>,
    /// Çekirdek ızgarası: (değer, çubuk).
    grid: Vec<(Label, Progress)>,
    rows: Vec<Row>,
    note: Option<Label>,
}

/// Bir bölümün içeriği (görünüm kurulumundan bağımsız, saf): testler ve güncelleme bunu kullanır.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionData {
    pub icon: IconName,
    pub title: String,
    pub value: String,
    /// Başlık altındaki geçmiş grafiği (servis geçmişinden).
    pub spark: Option<Metric>,
    pub bar: Option<f32>,
    pub grid: Vec<(String, f32)>,
    /// (sol, sağ, ek, çubuk).
    pub rows: Vec<(String, String, Option<String>, Option<f32>)>,
    pub note: Option<String>,
}

impl SectionData {
    fn new(icon: IconName, title: String, value: String) -> Self {
        Self { icon, title, value, spark: None, bar: None, grid: Vec::new(), rows: Vec::new(), note: None }
    }
}

/// 0..=1 kesir.
fn frac(pct: f32) -> f32 {
    if pct.is_finite() { (pct / 100.0).clamp(0.0, 1.0) } else { 0.0 }
}

fn decimal1(v: f32) -> String {
    format::fixed(f64::from(v), 2)
}

/// Ölçümden bölümler: (sol sütun, sağ sütun, alt). Olmayan ölçümün bölümü yok.
pub fn sections(s: &Snapshot) -> (Vec<SectionData>, Vec<SectionData>, Vec<SectionData>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut bottom = Vec::new();

    // İşlemci
    let mut cpu = SectionData::new(IconName::Cpu, t!("shell-sysmon-title-cpu"), s.cpu.map_or_else(|| "—".to_owned(), format::percent));
    cpu.spark = Some(Metric::Cpu);
    cpu.bar = Some(frac(s.cpu.unwrap_or(0.0)));
    cpu.grid = s.cores.iter().take(MAX_CORE_BARS).map(|&c| (format::percent(c), frac(c))).collect();
    let mut note = Vec::new();
    if s.cores.len() > MAX_CORE_BARS {
        note.push(t!("shell-sysmon-note-extra-cores", count = (s.cores.len() - MAX_CORE_BARS) as i64));
    }
    if let Some([a, b, c]) = s.load {
        note.push(t!("shell-sysmon-note-load", load1 = decimal1(a), load5 = decimal1(b), load15 = decimal1(c)));
    }
    if let Some(mhz) = s.freq_mhz {
        note.push(format!("{} GHz", format::fixed(f64::from(mhz) / 1000.0, 1)));
    }
    cpu.note = (!note.is_empty()).then(|| note.join(" · "));
    left.push(cpu);

    // Bellek
    if let Some(m) = s.memory {
        let u = Usage { used: m.used, total: m.total };
        let mut mem = SectionData::new(IconName::Memory, t!("shell-sysmon-title-memory"), format::usage(u));
        mem.spark = Some(Metric::Memory);
        mem.bar = Some(frac(u.percent()));
        if m.swap_total > 0 {
            mem.note = Some(t!("shell-sysmon-note-swap", usage = format::usage(Usage { used: m.swap_used, total: m.swap_total })));
        }
        left.push(mem);
    }

    // GPU'lar
    for (i, g) in s.gpus.iter().take(2).enumerate() {
        let title = format!("GPU · {}", g.driver);
        let value = if g.suspended { t!("shell-sysmon-gpu-suspended") } else { g.busy.map_or_else(|| "—".to_owned(), format::percent) };
        // Servis geçmişi yalnızca ilk GPU'yu izler (uykudayken boşluk).
        let spark = (i == 0).then_some(Metric::Gpu);
        let mut gpu = SectionData { title, spark, ..SectionData::new(IconName::Gpu, String::new(), value) };
        if !g.suspended {
            gpu.bar = g.busy.map(frac);
            gpu.note = g.vram.map(|v| format!("VRAM {}", format::usage(v)));
        }
        left.push(gpu);
    }

    // Sıcaklık
    if s.cpu_temp.is_some() || !s.temps.is_empty() {
        let value = s.cpu_temp.map(format::celsius).unwrap_or_default();
        let mut t = SectionData::new(IconName::Temperature, t!("shell-sysmon-title-temperature"), value);
        t.rows = s.temps.iter().take(MAX_ROWS).map(|x| (x.label.clone(), format::celsius(x.celsius), None, None)).collect();
        right.push(t);
    }

    // Ağ
    if let Some(n) = s.net {
        let mut net = SectionData::new(IconName::Download, t!("shell-sysmon-title-network"), String::new());
        net.rows = vec![
            (t!("shell-sysmon-title-download"), format::rate(n.down), None, None),
            (t!("shell-sysmon-title-upload"), format::rate(n.up), None, None),
        ];
        right.push(net);
    }

    // Disk
    if s.root.is_some() || !s.mounts.is_empty() || s.disk_io.is_some() {
        let value = s.root.map(|r| format::percent(r.percent())).unwrap_or_default();
        let mut d = SectionData::new(IconName::Disk, t!("shell-sysmon-title-disk"), value);
        if s.mounts.is_empty() {
            d.bar = s.root.map(|r| frac(r.percent()));
        }
        d.rows = s
            .mounts
            .iter()
            .take(MAX_ROWS.min(4))
            .map(|(m, u)| (m.clone(), format::usage(*u), None, Some(frac(u.percent()))))
            .collect();
        d.note = s.disk_io.map(|io| t!("shell-sysmon-note-disk-io", read = format::rate(io.down), write = format::rate(io.up)));
        right.push(d);
    }

    // Süreçler (yalnızca ayrıntı kipinde dolu)
    if !s.processes.is_empty() {
        let mut p = SectionData::new(IconName::Performance, t!("shell-sysmon-title-top-processes"), String::new());
        p.rows = s
            .processes
            .iter()
            .map(|p| (p.name.clone(), p.cpu.map_or_else(|| "—".to_owned(), format::percent), Some(format::bytes(p.rss)), None))
            .collect();
        bottom.push(p);
    }
    (left, right, bottom)
}

fn title_style(theme: &Theme) -> nuxut::TextStyle {
    text_style(theme, TextRole::Label).with_max_lines(1)
}

fn value_style(theme: &Theme) -> nuxut::TextStyle {
    let mut s = text_style(theme, TextRole::Body).with_tabular_numbers();
    s.align = TextAlign::End;
    s
}

fn caption_style(theme: &Theme) -> nuxut::TextStyle {
    text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1)
}

impl Row {
    fn new(theme: &Theme) -> Self {
        Self { left: Label::new("", caption_style(theme)), right: Label::new("", caption_style(theme)), extra: None, bar: None }
    }

    fn update(&mut self, theme: &Theme, data: &(String, String, Option<String>, Option<f32>), now: Instant) {
        self.left.set_text(&data.0);
        self.right.set_text(&data.1);
        match (&mut self.extra, &data.2) {
            (Some(l), Some(t)) => {
                l.set_text(t);
            }
            (e @ None, Some(t)) => *e = Some(Label::new(t.clone(), caption_style(theme))),
            (e, None) => *e = None,
        }
        sync_bar(&mut self.bar, theme, data.3, now);
    }
}

fn sync_bar(bar: &mut Option<Progress>, theme: &Theme, value: Option<f32>, now: Instant) {
    match (bar.as_mut(), value) {
        (Some(p), Some(v)) => p.set_value(v, now),
        (None, Some(v)) => *bar = Some(Progress::new(theme, v).with_threshold(usage_threshold())),
        (_, None) => *bar = None,
    }
}

impl Section {
    fn new(theme: &Theme, d: &SectionData, history: &History, now: Instant) -> Self {
        let mut s = Self {
            icon: IconView::with_role(theme, Icon::named(d.icon), TextRole::IconSmall),
            title: Label::new("", title_style(theme)),
            value: Label::new("", value_style(theme)),
            spark: None,
            bar: None,
            grid: Vec::new(),
            rows: Vec::new(),
            note: None,
        };
        s.update(theme, d, history, now);
        s
    }

    fn update(&mut self, theme: &Theme, d: &SectionData, history: &History, now: Instant) {
        self.icon.set_icon(Icon::named(d.icon));
        self.title.set_text(&d.title);
        self.value.set_text(&d.value);
        match (&mut self.spark, d.spark) {
            (Some(sp), Some(m)) if sp.metric == m => {}
            (sp, m) => *sp = m.map(|m| Spark::new(theme, m)),
        }
        if let Some(sp) = &mut self.spark {
            sp.sync(history, now);
        }
        sync_bar(&mut self.bar, theme, d.bar, now);
        self.grid.truncate(d.grid.len());
        for (i, (text, v)) in d.grid.iter().enumerate() {
            match self.grid.get_mut(i) {
                Some((l, p)) => {
                    l.set_text(text);
                    p.set_value(*v, now);
                }
                None => self.grid.push((
                    Label::new(text.clone(), caption_style(theme)),
                    Progress::new(theme, *v).with_threshold(usage_threshold()),
                )),
            }
        }
        self.rows.truncate(d.rows.len());
        for (i, r) in d.rows.iter().enumerate() {
            if self.rows.len() <= i {
                self.rows.push(Row::new(theme));
            }
            self.rows[i].update(theme, r, now);
        }
        match (&mut self.note, &d.note) {
            (Some(l), Some(t)) => {
                l.set_text(t);
            }
            (n @ None, Some(t)) => *n = Some(Label::new(t.clone(), caption_style(theme))),
            (n, None) => *n = None,
        }
    }

    fn row_height(&mut self, text: &mut TextSystem, theme: &Theme, row: usize) -> f32 {
        let r = &mut self.rows[row];
        let h = r.left.measure(text).height.max(r.right.measure(text).height);
        if r.bar.is_some() { h + theme.space(1) + bar_h(theme) } else { h }
    }

    fn height(&mut self, text: &mut TextSystem, theme: &Theme) -> f32 {
        let mut h = self.header_height(text);
        if self.spark.is_some() {
            h += theme.space(2) + spark_h(theme);
        }
        if self.bar.is_some() {
            h += theme.space(2) + bar_h(theme);
        }
        if !self.grid.is_empty() {
            let cell = self.grid[0].0.measure(text).height + theme.space(1) + bar_h(theme);
            let rows = self.grid.len().div_ceil(CORE_COLUMNS);
            h += theme.space(3) + rows as f32 * cell + (rows.saturating_sub(1)) as f32 * theme.space(2);
        }
        if !self.rows.is_empty() {
            h += theme.space(2);
            for i in 0..self.rows.len() {
                h += self.row_height(text, theme, i);
            }
            h += theme.space(1) * self.rows.len().saturating_sub(1) as f32;
        }
        if let Some(n) = &mut self.note {
            h += theme.space(2) + n.measure(text).height;
        }
        h
    }

    fn header_height(&mut self, text: &mut TextSystem) -> f32 {
        self.icon.measure(text).height.max(self.title.measure(text).height).max(self.value.measure(text).height)
    }

    fn draw(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) {
        let colors = &theme.colors;
        let on_surface = ui.fade(colors.on_surface);
        let variant = ui.fade(colors.on_surface_variant);
        let primary = ui.fade(colors.primary);
        let hh = self.header_height(ui.painter.text_system());
        let cy = area.y + hh * 0.5;
        let isz = self.icon.size().width;
        self.icon.draw_centered(ui.painter, Point::new(area.x + isz * 0.5, cy), primary);
        let vs = self.value.measure_in(ui.painter);
        let tx = area.x + isz + theme.space(2);
        self.title.set_max_width(Some((area.right() - vs.width - theme.space(2) - tx).max(0.0)));
        let ts = self.title.measure_in(ui.painter);
        self.title.draw(ui.painter, Point::new(tx, cy - ts.height * 0.5), on_surface);
        self.value.draw(ui.painter, Point::new(area.right() - vs.width, cy - vs.height * 0.5), on_surface);
        let mut y = area.y + hh;
        if let Some(sp) = &mut self.spark {
            y += theme.space(2);
            sp.line.set_rect(Rect::new(area.x, y, area.width, spark_h(theme)));
            sp.line.draw(ui);
            y += spark_h(theme);
        }
        if let Some(bar) = &mut self.bar {
            y += theme.space(2);
            bar.set_rect(Rect::new(area.x, y, area.width, bar_h(theme)));
            bar.draw(ui);
            y += bar_h(theme);
        }
        if !self.grid.is_empty() {
            y += theme.space(3);
            let gap = theme.space(3);
            let cw = ((area.width - gap * (CORE_COLUMNS - 1) as f32) / CORE_COLUMNS as f32).max(0.0);
            let lh = self.grid[0].0.measure_in(ui.painter).height;
            let cell = lh + theme.space(1) + bar_h(theme);
            for (i, (label, bar)) in self.grid.iter_mut().enumerate() {
                let (col, row) = (i % CORE_COLUMNS, i / CORE_COLUMNS);
                let x = area.x + col as f32 * (cw + gap);
                let cy = y + row as f32 * (cell + theme.space(2));
                label.draw(ui.painter, Point::new(x, cy), variant);
                bar.set_rect(Rect::new(x, cy + lh + theme.space(1), cw, bar_h(theme)));
                bar.draw(ui);
            }
            let rows = self.grid.len().div_ceil(CORE_COLUMNS);
            y += rows as f32 * cell + rows.saturating_sub(1) as f32 * theme.space(2);
        }
        if !self.rows.is_empty() {
            y += theme.space(2);
            // Ek sütun (süreç belleği) sabit genişlikte sağda; değer onun solunda.
            let extra_w = theme.space(18);
            let value_w = theme.space(12);
            for r in &mut self.rows {
                let rs = r.right.measure_in(ui.painter);
                let h = r.left.measure_in(ui.painter).height.max(rs.height);
                let mut right_edge = area.right();
                if let Some(e) = &mut r.extra {
                    let es = e.measure_in(ui.painter);
                    e.draw(ui.painter, Point::new(right_edge - es.width, y), variant);
                    right_edge -= extra_w;
                }
                r.right.draw(ui.painter, Point::new(right_edge - rs.width, y), on_surface);
                let left_max = if r.extra.is_some() { right_edge - value_w - area.x } else { right_edge - rs.width - theme.space(2) - area.x };
                r.left.set_max_width(Some(left_max.max(0.0)));
                r.left.draw(ui.painter, Point::new(area.x, y), variant);
                y += h;
                if let Some(bar) = &mut r.bar {
                    y += theme.space(1);
                    bar.set_rect(Rect::new(area.x, y, area.width, bar_h(theme)));
                    bar.draw(ui);
                    y += bar_h(theme);
                }
                y += theme.space(1);
            }
            y -= theme.space(1);
        }
        if let Some(n) = &mut self.note {
            y += theme.space(2);
            n.set_max_width(Some(area.width));
            n.draw(ui.painter, Point::new(area.x, y), variant);
        }
    }
}

fn bar_h(theme: &Theme) -> f32 {
    theme.space(1)
}

/// Geçmiş grafiğinin yüksekliği (iki sütunlu düzende framework varsayılanından, `space(10)`,
/// biraz alçak: üç grafik + çubuklar sol sütunu sağdan uzun tutmasın).
fn spark_h(theme: &Theme) -> f32 {
    theme.space(8)
}

#[derive(Debug)]
pub struct SysmonView {
    left: Vec<Section>,
    right: Vec<Section>,
    bottom: Vec<Section>,
    /// Bölüm kimlikleri (başlıklar): değişince bileşenler yeniden kurulur.
    keys: Vec<String>,
    snapshot: Option<Arc<Snapshot>>,
    waiting: Label,
}

impl SysmonView {
    pub fn new(theme: &Theme) -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
            bottom: Vec::new(),
            keys: Vec::new(),
            snapshot: None,
            waiting: Label::new(t!("shell-sysmon-waiting"), caption_style(theme)),
        }
    }

    fn inset(theme: &Theme) -> f32 {
        theme.space(5)
    }

    fn column_gap(theme: &Theme) -> f32 {
        theme.space(6)
    }

    fn section_gap(theme: &Theme) -> f32 {
        theme.space(5)
    }

    /// Yeni ölçüm. Görünen bir şey değiştiyse `true`.
    pub fn set_snapshot(&mut self, theme: &Theme, snapshot: Option<&Arc<Snapshot>>, now: Instant) -> bool {
        match (snapshot, &self.snapshot) {
            (Some(a), Some(b)) if Arc::ptr_eq(a, b) => return false,
            (None, None) => return false,
            _ => {}
        }
        self.snapshot = snapshot.cloned();
        self.rebuild(theme, now, false);
        true
    }

    fn rebuild(&mut self, theme: &Theme, now: Instant, force: bool) {
        let Some(s) = self.snapshot.clone() else {
            self.left.clear();
            self.right.clear();
            self.bottom.clear();
            self.keys.clear();
            return;
        };
        let (l, r, b) = sections(&s);
        let keys: Vec<String> = l.iter().chain(&r).chain(&b).map(|d| d.title.clone()).collect();
        if force || keys != self.keys {
            self.left = l.iter().map(|d| Section::new(theme, d, &s.history, now)).collect();
            self.right = r.iter().map(|d| Section::new(theme, d, &s.history, now)).collect();
            self.bottom = b.iter().map(|d| Section::new(theme, d, &s.history, now)).collect();
            self.keys = keys;
            return;
        }
        for (sec, d) in self.left.iter_mut().zip(&l).chain(self.right.iter_mut().zip(&r)).chain(self.bottom.iter_mut().zip(&b)) {
            sec.update(theme, d, &s.history, now);
        }
    }

    /// Dil değişti: görünüm yeniden kurulur, son ölçüm yeni metinlerle yeniden biçimlenir
    /// (görünümün kendi durumu yok; her şey ölçümden türer).
    pub fn relocalize(&mut self, theme: &Theme, now: Instant) {
        let snapshot = self.snapshot.take();
        *self = Self::new(theme);
        self.set_snapshot(theme, snapshot.as_ref(), now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.waiting.set_style(caption_style(theme));
        self.rebuild(theme, now, true);
    }

    fn column_width(theme: &Theme, width: f32) -> f32 {
        ((width - 2.0 * Self::inset(theme) - Self::column_gap(theme)) * 0.5).max(0.0)
    }

    fn stack_height(sections: &mut [Section], text: &mut TextSystem, theme: &Theme) -> f32 {
        let n = sections.len();
        sections.iter_mut().map(|s| s.height(text, theme)).sum::<f32>() + Self::section_gap(theme) * n.saturating_sub(1) as f32
    }

    /// İçerik yüksekliği (verilen genişlikte).
    pub fn preferred_height(&mut self, text: &mut TextSystem, theme: &Theme) -> f32 {
        let inset = Self::inset(theme);
        if self.snapshot.is_none() {
            return inset * 2.0 + self.waiting.measure(text).height;
        }
        let cols = Self::stack_height(&mut self.left, text, theme).max(Self::stack_height(&mut self.right, text, theme));
        let bottom = Self::stack_height(&mut self.bottom, text, theme);
        let gap = if self.bottom.is_empty() { 0.0 } else { Self::section_gap(theme) };
        inset * 2.0 + cols + gap + bottom
    }

    pub fn draw(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) {
        let inset = Self::inset(theme);
        if self.snapshot.is_none() {
            let c = ui.fade(theme.colors.on_surface_variant);
            self.waiting.draw(ui.painter, Point::new(area.x + inset, area.y + inset), c);
            return;
        }
        let cw = Self::column_width(theme, area.width);
        let gap = Self::section_gap(theme);
        let mut bottom_y = area.y + inset;
        for (x, col) in [(area.x + inset, &mut self.left), (area.x + inset + cw + Self::column_gap(theme), &mut self.right)] {
            let mut y = area.y + inset;
            for s in col.iter_mut() {
                let h = s.height(ui.painter.text_system(), theme);
                s.draw(ui, Rect::new(x, y, cw, h), theme);
                y += h + gap;
            }
            bottom_y = bottom_y.max(y);
        }
        let full = (area.width - 2.0 * inset).max(0.0);
        let mut y = bottom_y;
        for s in &mut self.bottom {
            let h = s.height(ui.painter.text_system(), theme);
            s.draw(ui, Rect::new(area.x + inset, y, full, h), theme);
            y += h + gap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::sysmon::{Gpu, Memory, Process, Rates, Temperature};

    #[test]
    fn missing_metrics_have_no_sections() {
        let (l, r, b) = sections(&Snapshot::default());
        assert_eq!(l.len(), 1, "işlemci her zaman (— ile)");
        assert_eq!(l[0].value, "—");
        assert!(r.is_empty() && b.is_empty());
    }

    #[test]
    fn full_snapshot_sections() {
        crate::i18n::use_turkish_for_tests();
        crate::percent::tr_catalog();
        let s = Snapshot {
            cpu: Some(25.0),
            cores: vec![10.0; 40],
            load: Some([0.5, 0.25, 1.0]),
            freq_mhz: Some(3200),
            memory: Some(Memory { total: 16 << 30, used: 4 << 30, swap_total: 8 << 30, swap_used: 0 }),
            gpus: vec![Gpu { driver: "amdgpu".into(), suspended: true, busy: None, vram: None }],
            cpu_temp: Some(50.0),
            temps: (0..10).map(|i| Temperature { label: format!("t{i}"), celsius: 40.0 }).collect(),
            net: Some(Rates { down: 1024.0, up: 0.0 }),
            disk_io: Some(Rates::default()),
            root: Some(Usage { used: 1, total: 4 }),
            mounts: vec![("/".into(), Usage { used: 1, total: 4 })],
            processes: vec![Process { pid: 1, name: "a".into(), cpu: Some(5.0), rss: 1 << 20 }],
            detail: true,
            history: History::default(),
        };
        let (l, r, b) = sections(&s);
        assert_eq!(l.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), ["İşlemci", "Bellek", "GPU · amdgpu"]);
        assert_eq!(l[0].grid.len(), MAX_CORE_BARS);
        assert_eq!(l[0].note.as_deref(), Some("+8 çekirdek · Yük 0,50 0,25 1,00 · 3,2 GHz"));
        assert_eq!(l[1].note.as_deref(), Some("Takas 0,0 / 8,0 GiB"));
        assert_eq!((l[2].value.as_str(), l[2].bar), ("uykuda", None));
        assert_eq!(r.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), ["Sıcaklık", "Ağ", "Disk"]);
        assert_eq!(r[0].rows.len(), MAX_ROWS);
        assert_eq!(r[2].bar, None, "bağlama noktası varsa satır çubukları");
        assert_eq!(r[2].rows[0].3, Some(0.25));
        assert_eq!(b[0].rows[0], ("a".into(), "%5".into(), Some("1,0 MiB".into()), None));
    }

    #[test]
    fn view_updates_in_place_and_measures() {
        crate::i18n::use_turkish_for_tests();
        crate::percent::tr_catalog();
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut v = SysmonView::new(&theme);
        let now = Instant::now();
        let h0 = v.preferred_height(&mut text, &theme);
        let s = Arc::new(Snapshot { cpu: Some(5.0), cores: vec![1.0, 2.0], ..Snapshot::default() });
        assert!(v.set_snapshot(&theme, Some(&s), now));
        assert!(!v.set_snapshot(&theme, Some(&s), now), "aynı ölçüm");
        let h1 = v.preferred_height(&mut text, &theme);
        assert!(h1 > h0);
        let s2 = Arc::new(Snapshot { cpu: Some(6.0), cores: vec![1.0, 3.0], ..Snapshot::default() });
        v.set_snapshot(&theme, Some(&s2), now);
        assert_eq!(v.left.len(), 1);
        assert_eq!(v.left[0].value.text(), "%6");
        assert_eq!(v.preferred_height(&mut text, &theme), h1, "aynı yapı, aynı yükseklik");
    }

    #[test]
    fn usage_bars_use_framework_threshold() {
        use nuxut::theme::ColorRole;
        use nuxut::ui::Tint;
        let theme = Theme::default();
        let now = Instant::now();
        assert_eq!(usage_threshold().tint, Tint::Role(ColorRole::Error), "renk tema rolü, gömülü değil");
        assert_eq!(spark_threshold().tint, Tint::Role(ColorRole::Error));
        assert_eq!(usage_threshold().level * 100.0, spark_threshold().level, "çubuk ve grafik aynı eşikte");
        let mut bar = None;
        sync_bar(&mut bar, &theme, Some(0.5), now);
        let p = bar.as_mut().unwrap();
        assert!(!p.is_alert());
        p.set_value(frac(95.0), now);
        assert!(p.is_alert(), "%95 → eşik rengi");
        p.set_value(frac(88.5), now);
        assert!(p.is_alert(), "geri dönüş payı içinde");
        p.set_value(frac(80.0), now);
        assert!(!p.is_alert());
        // Çekirdek çubukları da eşikli.
        let mut v = SysmonView::new(&theme);
        let s = Arc::new(Snapshot { cpu: Some(95.0), cores: vec![99.0, 1.0], ..Snapshot::default() });
        v.set_snapshot(&theme, Some(&s), now);
        let cpu = &v.left[0];
        assert!(cpu.bar.as_ref().unwrap().is_alert());
        assert_eq!(cpu.grid.iter().map(|(_, p)| p.is_alert()).collect::<Vec<_>>(), [true, false]);
    }

    #[test]
    fn sparks_follow_service_history() {
        let theme = Theme::default();
        let now = Instant::now();
        let mut h = History::default();
        let snap = |h: &History, cpu: f32| {
            Arc::new(Snapshot {
                cpu: Some(cpu),
                memory: Some(Memory { total: 100, used: 40, swap_total: 0, swap_used: 0 }),
                gpus: vec![
                    Gpu { driver: "amdgpu".into(), suspended: false, busy: Some(10.0), vram: None },
                    Gpu { driver: "i915".into(), suspended: false, busy: Some(5.0), vram: None },
                ],
                history: h.clone(),
                ..Snapshot::default()
            })
        };
        let mut v = SysmonView::new(&theme);
        for i in 0..3 {
            h.push(Some(i as f32), Some(40.0), Some(10.0));
        }
        v.set_snapshot(&theme, Some(&snap(&h, 2.0)), now);
        let sparks = |v: &SysmonView| v.left.iter().map(|s| s.spark.as_ref().map(|sp| sp.metric)).collect::<Vec<_>>();
        assert_eq!(sparks(&v), [Some(Metric::Cpu), Some(Metric::Memory), Some(Metric::Gpu), None], "yalnızca ilk GPU");
        let cpu_line = |v: &SysmonView| v.left[0].spark.as_ref().unwrap().line.history().iter().collect::<Vec<_>>();
        assert_eq!(cpu_line(&v), [0.0, 1.0, 2.0], "panel açılınca saklı geçmiş");
        // Ardışık ölçüm: yalnızca yeni örnek itilir.
        h.push(Some(50.0), Some(40.0), Some(10.0));
        v.set_snapshot(&theme, Some(&snap(&h, 50.0)), now);
        assert_eq!(cpu_line(&v), [0.0, 1.0, 2.0, 50.0]);
        // Aynı geçmiş (ayrıntı kipi ara ölçümü): değişmez.
        v.set_snapshot(&theme, Some(&snap(&h, 51.0)), now);
        assert_eq!(cpu_line(&v).len(), 4);
        // Atlanan ölçümler: tümü yeniden yüklenir; kapasite sınırı korunur.
        for i in 0..(HISTORY_LEN * 2) {
            h.push(Some((i % 100) as f32), None, None);
        }
        v.set_snapshot(&theme, Some(&snap(&h, 1.0)), now);
        assert_eq!(cpu_line(&v), h.series(Metric::Cpu).iter().collect::<Vec<_>>());
        assert_eq!(cpu_line(&v).len(), HISTORY_LEN);
        assert!(v.left[0].spark.as_ref().unwrap().line.history().capacity() == HISTORY_LEN);
        // Grafik eşiği son örnekte.
        h.push(Some(97.0), None, None);
        v.set_snapshot(&theme, Some(&snap(&h, 97.0)), now);
        assert!(v.left[0].spark.as_ref().unwrap().line.is_alert());
    }
}

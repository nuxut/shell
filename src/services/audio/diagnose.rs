//! `nuxut-shell msg audio-diagnose`: varsayılan çıkış/giriş için seviyenin hangi yoldan
//! okunup yazıldığı (route / Props) ve nedeni, route ile düğümün ham ses alanları ve
//! karşılaştırma için `wpctl get-volume` çıktısı. Kopyalanıp paylaşılmak üzere düz metindir;
//! cihaz/uygulama/medya **adı yazılmaz** (yalnızca id, sınıf ve port türü).

use std::fmt::Write as _;
use std::io::Read as _;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::AudioBackend;
use super::model::{Model, NodeKind, VolumePath, VolumeProps, peak, to_perceptual};
use super::writes::WriteState;

/// `wpctl` çağrısı için süre (IPC yanıtı 2 sn içinde gitmeli).
const WPCTL_TIMEOUT: Duration = Duration::from_millis(900);
/// `wpctl` çıktısından okunan en çok bayt.
const MAX_WPCTL_OUTPUT: u64 = 256;

/// Bir bölüm: (başlık, wpctl hedefi, gösterilen algısal seviye/sessiz).
pub(crate) type Section = (&'static str, &'static str, Option<(f32, bool)>);

/// Varsayılan bir düğümün seviye yolu, `msg doctor` özeti için (ad yok).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    /// Varsayılan düğüm yok (metadata'da anahtar yok ya da ad izlenmiyor).
    None,
    /// Kart cihazının etkin route'u (doğru yol).
    Route,
    /// Route listesi henüz okunmadı (yazım bekletiliyor).
    AwaitRoutes,
    /// Kart dışı düğüm `Props`'tan (doğru yol).
    Props,
    /// Kart düğümü `Props`'tan: yalnızca yazılım seviyesi değişir, donanım karıştırıcısı değişmez.
    CardProps,
}

/// `msg doctor` için ses özeti: rapor metninin ayrıştırılmaz özeti, aynı modelden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct AudioSummary {
    pub available: bool,
    /// `wpctl` | `native`
    pub backend: &'static str,
    pub sink: PathKind,
    pub source: PathKind,
}

/// Modelden ses özeti (`wpctl` çağrılmaz; ses thread'inde ucuz).
pub(crate) fn summary(model: &Model, backend: AudioBackend, available: bool) -> AudioSummary {
    let path = |kind: NodeKind| {
        let Some(id) = model.default_node(kind) else { return PathKind::None };
        let card = model.nodes[&id].info.device_id.is_some() && model.nodes[&id].info.profile_device.is_some();
        match model.path(id) {
            Some(VolumePath::Route { .. }) => PathKind::Route,
            Some(VolumePath::AwaitRoutes { .. }) => PathKind::AwaitRoutes,
            Some(VolumePath::Props(_)) if card => PathKind::CardProps,
            Some(VolumePath::Props(_)) | None => PathKind::Props,
        }
    };
    AudioSummary {
        available,
        backend: match backend {
            AudioBackend::Wpctl => "wpctl",
            AudioBackend::Native => "native",
        },
        sink: path(NodeKind::Sink),
        source: path(NodeKind::Source),
    }
}

/// Rapor başlığı ve iki düğüm bölümü (wpctl satırları sonra eklenir).
pub(crate) struct Report {
    pub text: String,
    pub sections: Vec<Section>,
}

fn list(v: &Option<Vec<f32>>) -> String {
    match v {
        None => "-".into(),
        Some(v) => {
            let items: Vec<String> = v.iter().map(|x| format!("{x:.4}")).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

fn percent(v: &Option<Vec<f32>>) -> String {
    match v {
        Some(v) if !v.is_empty() => format!(" (algısal %{:.1})", to_perceptual(peak(v)) * 100.0),
        _ => String::new(),
    }
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
}

fn props_line(out: &mut String, label: &str, p: &VolumeProps) {
    let _ = writeln!(
        out,
        "  {label}: channelVolumes={}{}{} softVolumes={} mute={}{} volume={} volumeBase={} volumeStep={}",
        list(&p.channels),
        percent(&p.channels),
        if p.hw_volume { " [donanım]" } else { "" },
        list(&p.soft),
        opt(p.mute),
        if p.hw_mute { " [donanım]" } else { "" },
        p.volume.map(|v| format!("{v:.4}")).unwrap_or_else(|| "-".into()),
        p.base.map(|v| format!("{v:.4}")).unwrap_or_else(|| "-".into()),
        p.step.map(|v| format!("{v:.6}")).unwrap_or_else(|| "-".into()),
    );
}

/// Modelden raporun gövdesi.
pub(crate) fn report(model: &Model, backend: AudioBackend, available: bool, private_remote: bool) -> Report {
    let mut out = String::new();
    let _ = writeln!(out, "nuxut-shell audio-diagnose ({})", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(
        out,
        "yazım yolu ([audio] backend): {}",
        match backend {
            AudioBackend::Wpctl => "wpctl (alt süreç; WirePlumber mixer-api)",
            AudioBackend::Native => "native (süreç içi; mixer-api ile aynı anlam)",
        }
    );
    let _ = writeln!(
        out,
        "pipewire: {}{}; izlenen: {} düğüm, {} ses cihazı; yazım: {} gönderildi, {} birleştirildi",
        if available { "bağlı" } else { "BAĞLI DEĞİL" },
        if private_remote { " (özel PIPEWIRE_REMOTE)" } else { "" },
        model.nodes.len(),
        model.devices.len(),
        model.writes.sent,
        model.writes.merged,
    );
    let mut sections = Vec::new();
    for (title, kind, target) in [
        ("varsayılan çıkış", NodeKind::Sink, "@DEFAULT_AUDIO_SINK@"),
        ("varsayılan giriş", NodeKind::Source, "@DEFAULT_AUDIO_SOURCE@"),
    ] {
        let _ = writeln!(out);
        let Some(id) = model.default_node(kind) else {
            let set = match kind {
                NodeKind::Sink => model.default_sink.is_some(),
                _ => model.default_source.is_some(),
            };
            let _ = writeln!(
                out,
                "{title}: yok ({})",
                if set { "metadata'daki ad izlenen düğümlerde yok" } else { "default metadata'sında anahtar yok" }
            );
            sections.push((title, target, None));
            continue;
        };
        let n = &model.nodes[&id];
        let _ = writeln!(out, "{title}: düğüm {id} ({})", n.kind.class());
        let _ = writeln!(
            out,
            "  device.id={} card.profile.device={}",
            opt(n.info.device_id),
            opt(n.info.profile_device)
        );
        if let Some(d) = n.info.device_id.and_then(|d| model.devices.get(&d)) {
            let _ = writeln!(
                out,
                "  cihaz: route listesi {} ({} numaralandırma, {} etkin route)",
                if d.committed { "okundu" } else { "HENÜZ OKUNMADI" },
                d.enumerations,
                d.routes.len()
            );
        }
        match model.path(id) {
            Some(VolumePath::Route { device, route }) => {
                let dir = if route.direction == pipewire::spa::sys::SPA_DIRECTION_OUTPUT { "output" } else { "input" };
                let _ = writeln!(
                    out,
                    "  yol: ROUTE — cihaz {device}, route index {}, profil cihazı {}, yön {dir}, port türü {}",
                    route.index,
                    route.device,
                    opt(route.port_type.as_deref())
                );
                let _ = writeln!(out, "  neden: kart düğümü ve cihazın bu profil cihazına ait etkin route'u var");
                props_line(&mut out, "route props", route.props.as_ref().unwrap_or(&VolumeProps::default()));
            }
            Some(VolumePath::AwaitRoutes { device }) => {
                let _ = writeln!(out, "  yol: BEKLİYOR — cihaz {device} route listesi henüz okunmadı (yazım bekletilir)");
            }
            Some(VolumePath::Props(reason)) => {
                let _ = writeln!(out, "  yol: PROPS — {}", reason.describe());
                if n.info.device_id.is_some() && n.info.profile_device.is_some() {
                    let _ = writeln!(
                        out,
                        "  UYARI: kart düğümü Props'tan yazılıyor: yalnızca yazılım seviyesi değişir, donanım karıştırıcısı değişmez"
                    );
                }
            }
            None => {}
        }
        props_line(&mut out, "düğüm Props", &n.props);
        if let (Some(VolumePath::Route { route, .. }), Some(nc)) = (model.path(id), &n.props.channels)
            && let Some(rc) = route.props.as_ref().and_then(|p| p.channels.as_ref())
            && (rc.len() != nc.len() || rc.iter().zip(nc).any(|(a, b)| (a - b).abs() > 1e-3))
        {
            let _ = writeln!(out, "  NOT: route ve düğüm channelVolumes farklı (düğüm route'un gecikmeli yansıması)");
        }
        let shown = model.volume(id);
        let _ = writeln!(
            out,
            "  gösterilen: {}",
            match shown {
                Some((v, m)) => format!("%{:.1}{}", v * 100.0, if m { " (sessiz)" } else { "" }),
                None => "seviye bilinmiyor".into(),
            }
        );
        let _ = writeln!(
            out,
            "  yazım durumu: {}",
            match model.writes.state(id) {
                None => "boşta",
                Some(WriteState::InFlight { pending: false }) => "yolda",
                Some(WriteState::InFlight { pending: true }) => "yolda + bekleyen",
                Some(WriteState::Parked) => "route bekleniyor",
                Some(WriteState::Settling) => "teyit bekleniyor",
            }
        );
        sections.push((title, target, shown));
    }
    Report { text: out, sections }
}

/// `wpctl get-volume <hedef>` başlatır (kabuk yok; özel PipeWire örneğine bağlıysak oraya).
fn spawn_wpctl(remote: Option<&str>, target: &str) -> Option<Child> {
    let mut cmd = Command::new("wpctl");
    cmd.args(["get-volume", target]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(r) = remote {
        cmd.env("PIPEWIRE_REMOTE", r);
    }
    cmd.spawn().ok()
}

/// Süre sınırlı bekler; çıktının ilk satırı ya da açıklama.
fn finish(child: Option<Child>, deadline: Instant) -> String {
    let Some(mut child) = child else { return "(wpctl çalıştırılamadı)".into() };
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return "(wpctl zaman aşımı)".into();
            }
        }
    }
    let mut text = String::new();
    if let Some(out) = child.stdout.take() {
        let _ = out.take(MAX_WPCTL_OUTPUT).read_to_string(&mut text);
    }
    let line = text.lines().next().unwrap_or("").trim();
    // Beklenen biçim yalnızca sayı ve [MUTED]; başka bir şey (hata metni) aynen ama sınırlı.
    if line.is_empty() { "(çıktı yok — wpctl hata verdi)".into() } else { line.chars().filter(|c| !c.is_control()).collect() }
}

/// `Volume: 0.50 [MUTED]` → (0.50, sessiz).
pub(crate) fn parse_wpctl(line: &str) -> Option<(f32, bool)> {
    let rest = line.strip_prefix("Volume:")?.trim();
    let mut parts = rest.split_whitespace();
    let v: f32 = parts.next()?.parse().ok()?;
    Some((v, parts.any(|p| p == "[MUTED]")))
}

/// Rapora `wpctl` satırlarını ve uyum kontrolünü ekler (bloklar; ayrı thread'de çağrılır).
pub(crate) fn complete(report: Report, remote: Option<&str>) -> String {
    let Report { mut text, sections } = report;
    let deadline = Instant::now() + WPCTL_TIMEOUT;
    let children: Vec<_> = sections.iter().map(|(_, target, _)| spawn_wpctl(remote, target)).collect();
    let _ = writeln!(text);
    for ((title, target, shown), child) in sections.into_iter().zip(children) {
        let line = finish(child, deadline);
        let _ = writeln!(text, "wpctl get-volume {target} ({title}): {line}");
        if let (Some((v, m)), Some((wv, wm))) = (shown, parse_wpctl(&line)) {
            let same = (v - wv).abs() < 0.006 && m == wm;
            let _ = writeln!(
                text,
                "  uyum: {}",
                if same { "evet".to_string() } else { format!("HAYIR (shell %{:.1}, wpctl %{:.1})", v * 100.0, wv * 100.0) }
            );
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audio::model::{NodeEntry, NodeInfo, Route};

    #[test]
    fn wpctl_output_parsed() {
        assert_eq!(parse_wpctl("Volume: 0.50"), Some((0.5, false)));
        assert_eq!(parse_wpctl("Volume: 1.00 [MUTED]"), Some((1.0, true)));
        assert_eq!(parse_wpctl("Translate ID error"), None);
    }

    #[test]
    fn report_explains_path_without_names() {
        let mut m = Model::default();
        let info = NodeInfo::parse(|k| match k {
            "node.name" => Some("alsa_output.gizli-kart-adi"),
            "node.description" => Some("Gizli Kart"),
            "device.id" => Some("9"),
            "card.profile.device" => Some("1"),
            _ => None,
        });
        m.add_node(10, NodeEntry::new(NodeKind::Sink, info));
        m.default_sink = Some("alsa_output.gizli-kart-adi".into());
        m.add_device(9);
        let r = report(&m, AudioBackend::Native, true, false);
        assert!(r.text.contains("yol: BEKLİYOR"), "{}", r.text);
        m.begin_routes(9, 1);
        let props = VolumeProps { channels: Some(vec![0.125, 0.125]), mute: Some(false), hw_volume: true, ..Default::default() };
        m.add_route(
            9,
            1,
            Route {
                index: 4,
                direction: pipewire::spa::sys::SPA_DIRECTION_OUTPUT,
                device: 1,
                props: Some(props),
                port_type: Some("speaker".into()),
            },
        );
        m.commit_routes(9, 1);
        m.set_props(10, VolumeProps { channels: Some(vec![1.0, 1.0]), ..Default::default() });
        let r = report(&m, AudioBackend::Native, true, false);
        let t = &r.text;
        assert!(t.contains("yol: ROUTE — cihaz 9, route index 4, profil cihazı 1, yön output, port türü speaker"), "{t}");
        assert!(t.contains("[0.1250, 0.1250] (algısal %50.0) [donanım]"), "{t}");
        assert!(t.contains("route ve düğüm channelVolumes farklı"), "{t}");
        assert!(t.contains("gösterilen: %50.0"), "{t}");
        assert!(t.contains("varsayılan giriş: yok"), "{t}");
        assert!(!t.contains("gizli") && !t.contains("Gizli"), "ad sızmamalı: {t}");
        assert_eq!(r.sections[0].2, Some((0.5, false)));
        // Kart düğümü ama route'suz profil → Props uyarısı.
        m.begin_routes(9, 2);
        m.commit_routes(9, 2);
        let t = report(&m, AudioBackend::Wpctl, true, false).text;
        assert!(t.contains("yol: PROPS — cihazın etkin route'ları") && t.contains("UYARI"), "{t}");
    }
}

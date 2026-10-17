//! Algılamadan plan: her hedef için durum, notlar ve değişecek yolların listesi.
//!
//! Kullanıcı tarafı bunu yetki istemeden önce gösterir ("neyin değişeceği önceden listelenir");
//! kök yardımcı kendi algılamasıyla aynı planı yeniden kurar ve yalnız onu uygular (kullanıcı
//! tarafının planına güvenmez). Metin yok: notlar ve değişiklikler tür olarak, kabuk çevirir.

use serde::{Deserialize, Serialize};

use crate::detect::{GrubInfo, Initramfs, LimineConf, PlymouthInfo, SystemInfo};
use crate::{grub, plymouth};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Target {
    Plymouth,
    Grub,
    Limine,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Plymouth, Target::Grub, Target::Limine];

    pub fn name(self) -> &'static str {
        match self {
            Self::Plymouth => "plymouth",
            Self::Grub => "grub",
            Self::Limine => "limine",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Uygulanabilir.
    Ready,
    /// Zaten uygulanmış (yeniden uygulama renkleri tazeler).
    Applied,
    /// Kurulu değil.
    Absent,
    /// Kurulu ama güvenle değiştirilemez (notlarda neden).
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum Note {
    /// Plymouth initramfs'te değil (HOOKS'ta yok): tema kurulur ama açılışın erken kısmında görünmez.
    PlymouthNotInInitramfs,
    /// Komut satırında `splash` yok: Plymouth açılışta görünmez.
    PlymouthNoSplash,
    /// initramfs elle yeniden kurulmalı (dracut/bilinmeyen üretici ya da çözülemeyen ön ayar).
    /// `command`: üreticinin adı (`mkinitcpio`, `dracut`; bilinmiyorsa boş). Alan adı sözleşme
    /// gereği kaldı; kullanıcıya kök komutu yazdırılmaz (kullanıcı kararı 2026-10-17).
    PlymouthManualRebuild { command: String },
    GrubTextTerminal,
    GrubHandWritten,
    GrubCfgUnreadable,
    GrubNoDropins,
    GrubOverridden { path: String },
    LimineAmbiguous { paths: Vec<String> },
    LimineUnreadable,
    LimineUnsupported { reason: String },
    LimineEnrolled { binary: String },
}

impl Note {
    /// Bu not hedefi engeller mi.
    pub fn blocks(&self) -> bool {
        matches!(
            self,
            Self::GrubHandWritten
                | Self::GrubNoDropins
                | Self::GrubOverridden { .. }
                | Self::LimineAmbiguous { .. }
                | Self::LimineUnsupported { .. }
                | Self::LimineEnrolled { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChangeKind {
    /// Yeni dosya/dizin (geri almada silinir).
    Create,
    /// Var olan dosya değişir (önce yedeklenir).
    Modify,
    /// Komut çalışır (çıktısı gösterilir).
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// Mutlak yol ya da komut satırı.
    pub what: String,
}

impl Change {
    fn create(p: impl Into<String>) -> Self {
        Self { kind: ChangeKind::Create, what: p.into() }
    }
    fn modify(p: impl Into<String>) -> Self {
        Self { kind: ChangeKind::Modify, what: p.into() }
    }
    fn run(p: impl Into<String>) -> Self {
        Self { kind: ChangeKind::Run, what: p.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetPlan {
    pub target: Target,
    pub state: State,
    pub notes: Vec<Note>,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub targets: Vec<TargetPlan>,
    /// systemd-boot bulundu: temalanamaz (yalnız metin menüsü; bkz. tasarım belgesi).
    pub systemd_boot: bool,
}

impl Plan {
    pub fn get(&self, t: Target) -> Option<&TargetPlan> {
        self.targets.iter().find(|p| p.target == t)
    }

    /// Uygulanabilir hedefler (hazır ya da zaten uygulanmış).
    pub fn actionable(&self) -> Vec<Target> {
        self.targets.iter().filter(|p| matches!(p.state, State::Ready | State::Applied)).map(|p| p.target).collect()
    }
}

fn abs(rel: &str) -> String {
    format!("/{rel}")
}

/// mkinitcpio'nun tam yolu (yardımcı ve plan aynı komutu gösterir).
pub const MKINITCPIO_CMD: &str = "/usr/bin/mkinitcpio -P";

fn plan_plymouth(info: &PlymouthInfo) -> TargetPlan {
    if !info.installed {
        return TargetPlan { target: Target::Plymouth, state: State::Absent, notes: vec![], changes: vec![] };
    }
    let mut notes = Vec::new();
    let mut changes = vec![Change::create(plymouth::THEME_DIR), Change::modify(abs(crate::detect::PLYMOUTH_CONF))];
    match &info.initramfs {
        Initramfs::Mkinitcpio { hook: true, images: Some(images) } => {
            changes.extend(images.iter().map(|i| Change::modify(i.clone())));
            changes.push(Change::run(MKINITCPIO_CMD));
        }
        Initramfs::Mkinitcpio { hook: true, images: None } => {
            notes.push(Note::PlymouthManualRebuild { command: "mkinitcpio".into() })
        }
        Initramfs::Mkinitcpio { hook: false, .. } => notes.push(Note::PlymouthNotInInitramfs),
        Initramfs::Dracut => notes.push(Note::PlymouthManualRebuild { command: "dracut".into() }),
        Initramfs::Unknown => notes.push(Note::PlymouthManualRebuild { command: String::new() }),
    }
    if !info.splash {
        notes.push(Note::PlymouthNoSplash);
    }
    let state = if info.current_theme.as_deref() == Some(plymouth::THEME_NAME) { State::Applied } else { State::Ready };
    TargetPlan { target: Target::Plymouth, state, notes, changes }
}

fn plan_grub(g: &GrubInfo) -> TargetPlan {
    let mut notes = Vec::new();
    match g.generated {
        Some(false) => notes.push(Note::GrubHandWritten),
        None => notes.push(Note::GrubCfgUnreadable),
        Some(true) => {}
    }
    if !g.dropins {
        notes.push(Note::GrubNoDropins);
    }
    if let Some(p) = &g.overridden_by {
        notes.push(Note::GrubOverridden { path: abs(p) });
    }
    if !g.graphical {
        notes.push(Note::GrubTextTerminal);
    }
    let cfg = abs(&format!("{}/grub.cfg", g.dir));
    let changes = vec![
        Change::create(abs(&format!("{}/{}", g.dir, grub::THEME_SUBDIR))),
        Change::create(format!("{}/{}", grub::DROPIN_DIR, grub::DROPIN_NAME)),
        Change::modify(cfg.clone()),
        Change::run(format!("/usr/bin/grub-mkconfig -o {cfg}")),
    ];
    let state = if notes.iter().any(Note::blocks) {
        State::Blocked
    } else if g.applied {
        State::Applied
    } else {
        State::Ready
    };
    TargetPlan { target: Target::Grub, state, notes, changes }
}

fn plan_limine(l: &LimineConf) -> TargetPlan {
    let (state, notes, changes) = match l {
        LimineConf::Ok { path, applied } => {
            (if *applied { State::Applied } else { State::Ready }, vec![], vec![Change::modify(abs(path))])
        }
        LimineConf::Unreadable { path } => (State::Ready, vec![Note::LimineUnreadable], vec![Change::modify(abs(path))]),
        LimineConf::Ambiguous { paths } => {
            (State::Blocked, vec![Note::LimineAmbiguous { paths: paths.iter().map(|p| abs(p)).collect() }], vec![])
        }
        LimineConf::Unsupported { reason, .. } => (State::Blocked, vec![Note::LimineUnsupported { reason: reason.clone() }], vec![]),
        LimineConf::Enrolled { binary, .. } => (State::Blocked, vec![Note::LimineEnrolled { binary: abs(binary) }], vec![]),
    };
    TargetPlan { target: Target::Limine, state, notes, changes }
}

/// Algılamadan plan.
pub fn build(info: &SystemInfo) -> Plan {
    let mut targets = vec![plan_plymouth(&info.plymouth)];
    targets.push(match &info.grub {
        Some(g) => plan_grub(g),
        None => TargetPlan { target: Target::Grub, state: State::Absent, notes: vec![], changes: vec![] },
    });
    targets.push(match &info.limine {
        Some(l) => plan_limine(l),
        None => TargetPlan { target: Target::Limine, state: State::Absent, notes: vec![], changes: vec![] },
    });
    Plan { targets, systemd_boot: info.systemd_boot }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{GrubInfo, PlymouthInfo};

    fn info() -> SystemInfo {
        SystemInfo {
            plymouth: PlymouthInfo {
                installed: true,
                current_theme: Some("bgrt".into()),
                initramfs: Initramfs::Mkinitcpio { hook: true, images: Some(vec!["/boot/initramfs-linux.img".into()]) },
                splash: true,
            },
            grub: Some(GrubInfo {
                dir: "boot/grub".into(),
                generated: Some(true),
                dropins: true,
                graphical: true,
                overridden_by: None,
                applied: false,
            }),
            limine: None,
            systemd_boot: false,
        }
    }

    #[test]
    fn ready_plan_lists_every_change() {
        let p = build(&info());
        assert_eq!(p.actionable(), vec![Target::Plymouth, Target::Grub]);
        let g = p.get(Target::Grub).unwrap();
        assert_eq!(g.state, State::Ready);
        assert!(g.changes.contains(&Change::create("/etc/default/grub.d/90-nuxut-theme.cfg")));
        assert!(g.changes.contains(&Change::run("/usr/bin/grub-mkconfig -o /boot/grub/grub.cfg")));
        let pl = p.get(Target::Plymouth).unwrap();
        assert!(pl.changes.contains(&Change::modify("/boot/initramfs-linux.img")));
        assert!(pl.changes.contains(&Change::run(MKINITCPIO_CMD)));
        assert_eq!(p.get(Target::Limine).unwrap().state, State::Absent);
    }

    #[test]
    fn blocked_and_notes() {
        let mut i = info();
        i.grub.as_mut().unwrap().generated = Some(false);
        i.plymouth.splash = false;
        i.plymouth.initramfs = Initramfs::Mkinitcpio { hook: false, images: None };
        i.limine = Some(LimineConf::Enrolled { path: "boot/limine.conf".into(), binary: "boot/EFI/BOOT/BOOTX64.EFI".into() });
        let p = build(&i);
        assert_eq!(p.get(Target::Grub).unwrap().state, State::Blocked);
        assert_eq!(p.get(Target::Limine).unwrap().state, State::Blocked);
        let pl = p.get(Target::Plymouth).unwrap();
        assert_eq!(pl.state, State::Ready);
        assert!(pl.notes.contains(&Note::PlymouthNoSplash) && pl.notes.contains(&Note::PlymouthNotInInitramfs));
        assert!(!pl.changes.iter().any(|c| c.kind == ChangeKind::Run), "hook yoksa initramfs kurulmaz");
        assert_eq!(p.actionable(), vec![Target::Plymouth]);
    }

    #[test]
    fn applied_state_and_serde() {
        let mut i = info();
        i.plymouth.current_theme = Some("nuxut".into());
        i.grub.as_mut().unwrap().applied = true;
        let p = build(&i);
        assert_eq!(p.get(Target::Plymouth).unwrap().state, State::Applied);
        assert_eq!(p.get(Target::Grub).unwrap().state, State::Applied);
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains(r#""target":"grub","state":"applied""#), "{json}");
        let back: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }
}

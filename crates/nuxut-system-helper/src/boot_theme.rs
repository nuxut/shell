//! `boot-theme apply | revert | status`: Plymouth, GRUB ve Limine temasını uygular/geri alır.
//!
//! Her hedef ayrı bir işlemdir ([`Tx`]): ya tamamen uygulanır ya da o turdaki tüm değişiklikleri
//! geri alınır; bir hedefin hatası diğerlerini etkilemez. Sıra GRUB → Limine → Plymouth (en
//! uzun süren initramfs en sonda). Plan yardımcının **kendi** algılamasıyla kurulur; kullanıcı
//! tarafından gelen yalnız renkler, ölçek, dil ve hedef listesidir.
//!
//! Önyükleyici yapılandırması yerine konmadan önce doğrulanır:
//! - GRUB: `grub-mkconfig` yeni dosyaya (`grub.cfg.nuxut-new`) yazar → `grub-script-check` →
//!   [`grub::check_new_cfg`] (üretilmiş başlık, giriş sayısı azalmamış, tema yükleniyor) → eski
//!   dosya yedeklenir → atomik yeniden adlandırma.
//! - Limine: tüm dosya ayrıştırılır, blok eklenir, sonuç yeniden ayrıştırılır; kayıtlı sağlama
//!   toplamı varsa hiç dokunulmaz.
//! - initramfs: imgeler önce yedeklenir (yer yoksa hedef uygulanmaz), mkinitcpio başarısızsa
//!   eski imgeler geri konur; başarıda işlem yedekleri silinir (/boot dolmasın).

use nuxut_boot_theme::detect::{self, Initramfs, LimineConf, PLYMOUTH_CONF, PLYMOUTH_SPINNER_KEYMAP, SystemInfo};
use nuxut_boot_theme::fontsel::{self, Chosen};
use nuxut_boot_theme::plan::{self, Plan, State, Target};
use nuxut_boot_theme::{BootColors, Lang, ThemeOptions, grub, limine, pf2, plymouth};

use crate::cmd::{self, Bins, Tool};
use crate::events::{Event, Events, Outcome, StepState};
use crate::manifest::{Entry, Manifest, hash};
use crate::ops::{OpError, Ops, OpsProbe, Result, Tx, swap_dir};
use crate::policy::{self, LOCK_DIR, LOCK_NAME, MANIFEST, PLYMOUTH_THEME, STATE_DIR};
use nuxut_boot_theme::grub::PF2_RANGES;

/// Uygulama sırası.
const ORDER: [Target; 3] = [Target::Grub, Target::Limine, Target::Plymouth];
/// initramfs yedeği için imgelerin toplamına ek pay.
const SPACE_MARGIN: u64 = 64 * 1024 * 1024;
const MAX_FONT_OUT: usize = 8 * 1024 * 1024;
const MAX_KEYMAP: usize = 4 * 1024 * 1024;

/// Yardımcı bağlamı.
#[derive(Debug)]
pub struct Helper {
    pub ops: Ops,
    pub bins: Bins,
    pub events: Events,
}

/// `apply` girdisi (doğrulanmış).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyRequest {
    pub targets: Vec<Target>,
    pub colors: BootColors,
    pub scale: u32,
    pub lang: Lang,
}

fn err(s: impl Into<String>) -> OpError {
    OpError::Other(s.into())
}

impl Helper {
    fn lock(&self) -> Result<std::os::fd::OwnedFd> {
        Ok(self.ops.root.dir(LOCK_DIR)?.lock_file(LOCK_NAME)?)
    }

    pub fn detect(&self) -> SystemInfo {
        let cmdline = self.ops.read("proc/cmdline", 64 * 1024).ok().flatten().unwrap_or_default();
        detect::system(&OpsProbe(&self.ops), &String::from_utf8_lossy(&cmdline))
    }

    fn manifest_rel() -> String {
        format!("{STATE_DIR}/{MANIFEST}")
    }

    fn load_manifest(&self) -> Result<Manifest> {
        match self.ops.read(&Self::manifest_rel(), 1 << 20)? {
            Some(b) => Ok(Manifest::parse(&b)?),
            None => Ok(Manifest { version: crate::manifest::VERSION, entries: vec![] }),
        }
    }

    fn save_manifest(&self, m: &Manifest) -> Result<()> {
        if m.entries.is_empty() {
            return self.ops.remove_file(&Self::manifest_rel());
        }
        self.ops.ensure_dir(STATE_DIR)?;
        self.ops.write(&Self::manifest_rel(), &m.to_bytes())
    }

    /// Kaydı ekler ve bildirimi hemen yazar (günlük).
    fn record(&self, m: &mut Manifest, e: Entry) -> Result<()> {
        m.upsert(e);
        self.save_manifest(m)
    }

    fn run(&self, tool: Tool, args: &[&str]) -> Result<()> {
        cmd::run(&self.bins, tool, args, &self.events).map_err(|e| err(e.to_string()))
    }

    /// Kök tarafında plan (yalnız algılama).
    pub fn status(&self) -> Plan {
        let p = plan::build(&self.detect());
        self.events.emit(Event::Plan { plan: p.clone() });
        p
    }

    pub fn apply(&self, req: &ApplyRequest) -> bool {
        let _lock = match self.lock() {
            Ok(l) => l,
            Err(e) => return self.fail_all(&e.to_string()),
        };
        let info = self.detect();
        let plan = plan::build(&info);
        self.events.emit(Event::Plan { plan: plan.clone() });
        let mut manifest = match self.load_manifest() {
            Ok(m) => m,
            Err(e) => return self.fail_all(&e.to_string()),
        };
        let fonts = fontsel::choose_fonts(&OpsProbe(&self.ops));
        let opts = ThemeOptions {
            colors: req.colors,
            scale: req.scale,
            lang: req.lang,
            family: fonts.as_ref().map_or("Sans", |f| f.0.family).to_owned(),
            mono_family: fonts.as_ref().map_or("monospace", |f| f.1.family).to_owned(),
        };
        let mut ok = true;
        let mut reboot = false;
        for t in ORDER.into_iter().filter(|t| req.targets.contains(t)) {
            let tp = plan.get(t);
            if !tp.is_some_and(|p| matches!(p.state, State::Ready | State::Applied)) {
                let why = tp.map(|p| format!("{:?}: {:?}", p.state, p.notes)).unwrap_or_default();
                self.events.emit(Event::Target { target: t, outcome: Outcome::Skipped, message: Some(why) });
                continue;
            }
            let snapshot = manifest.clone();
            let mut tx = Tx::default();
            let r = match t {
                Target::Grub => self.apply_grub(&info, &opts, fonts.as_ref(), &mut manifest, &mut tx),
                Target::Limine => self.apply_limine(&info, &opts, &mut manifest, &mut tx),
                Target::Plymouth => self.apply_plymouth(&info, &opts, &mut manifest, &mut tx),
            };
            ok &= self.finish(t, r, tx, &mut manifest, snapshot, Outcome::Applied);
            reboot = true;
        }
        self.events.emit(Event::Done { ok, reboot_hint: reboot && ok });
        ok
    }

    pub fn revert(&self, targets: &[Target]) -> bool {
        let _lock = match self.lock() {
            Ok(l) => l,
            Err(e) => return self.fail_all(&e.to_string()),
        };
        let info = self.detect();
        let mut manifest = match self.load_manifest() {
            Ok(m) => m,
            Err(e) => return self.fail_all(&e.to_string()),
        };
        let mut ok = true;
        let mut reboot = false;
        for t in ORDER.into_iter().filter(|t| targets.contains(t)) {
            let entries: Vec<Entry> = manifest.for_target(t).cloned().collect();
            if entries.is_empty() {
                self.events.emit(Event::Target { target: t, outcome: Outcome::Skipped, message: None });
                continue;
            }
            let snapshot = manifest.clone();
            let mut tx = Tx::default();
            let r = self.revert_target(t, &entries, &info, &mut tx);
            if r.is_ok() {
                manifest.remove_target(t);
            }
            ok &= self.finish(t, r, tx, &mut manifest, snapshot, Outcome::Reverted);
            reboot = true;
        }
        self.events.emit(Event::Done { ok, reboot_hint: reboot && ok });
        ok
    }

    fn fail_all(&self, message: &str) -> bool {
        self.events.warn(None, message);
        self.events.emit(Event::Done { ok: false, reboot_hint: false });
        false
    }

    /// İşlemi kapatır: başarıda temizlik + bildirim, hatada geri alma + bildirimin eski hâli.
    fn finish(&self, t: Target, r: Result<()>, tx: Tx, manifest: &mut Manifest, snapshot: Manifest, good: Outcome) -> bool {
        match r {
            Ok(()) => {
                for w in tx.commit(&self.ops) {
                    self.events.warn(Some(t), w);
                }
                if let Err(e) = self.save_manifest(manifest) {
                    self.events.warn(Some(t), e.to_string());
                }
                self.events.emit(Event::Target { target: t, outcome: good, message: None });
                true
            }
            Err(e) => {
                let errors = tx.rollback(&self.ops);
                *manifest = snapshot;
                let saved = self.save_manifest(manifest);
                let (outcome, message) = if errors.is_empty() && saved.is_ok() {
                    (Outcome::RolledBack, e.to_string())
                } else {
                    let mut m = e.to_string();
                    for x in errors.iter().chain(saved.err().map(|e| e.to_string()).iter()) {
                        m.push_str("; geri alma: ");
                        m.push_str(x);
                    }
                    (Outcome::Inconsistent, m)
                };
                self.events.emit(Event::Target { target: t, outcome, message: Some(message) });
                false
            }
        }
    }

    /// Dosyayı `new` yapar: yoksa kurar (kayıt `File`), bizim değilse önce kalıcı yedek alır
    /// (kayıt `Edit`), zaten bizimse yalnız işlem içi geri yazma tutar.
    fn edit_file(&self, t: Target, rel: &str, new: &[u8], m: &mut Manifest, tx: &mut Tx) -> Result<()> {
        let old = self.ops.read(rel, detect::MAX_GRUB_CFG)?;
        if old.is_some() {
            let (dir, name) = crate::safefs::split(rel)?;
            if !self.ops.root.dir(dir)?.owned_file(name)? {
                return Err(err(format!("/{rel}: kökün dosyası değil, dokunulmadı")));
            }
        }
        let known = m.find(rel).cloned();
        match (old, known) {
            (None, _) => {
                tx.on_fail_remove_file(rel);
                self.ops.write(rel, new)?;
                self.record(m, Entry::File { target: t, path: rel.to_owned(), hash: hash(new) })
            }
            (Some(old), Some(Entry::Edit { backup, .. })) => {
                tx.on_fail_write(rel, old);
                self.ops.write(rel, new)?;
                self.record(m, Entry::Edit { target: t, path: rel.to_owned(), backup, hash: hash(new) })
            }
            (Some(old), Some(Entry::File { .. })) => {
                tx.on_fail_write(rel, old);
                self.ops.write(rel, new)?;
                self.record(m, Entry::File { target: t, path: rel.to_owned(), hash: hash(new) })
            }
            (Some(old), _) => {
                let backup = self.ops.backup(rel)?.ok_or_else(|| err(format!("/{rel}: yedeklenemedi")))?;
                tx.on_fail_remove_file(&backup);
                tx.on_fail_write(rel, old);
                self.ops.write(rel, new)?;
                self.record(m, Entry::Edit { target: t, path: rel.to_owned(), backup, hash: hash(new) })
            }
        }
    }

    /// Tema dizinini kurar; bildirimde zaten bizimse eski hâli silinir, değilse kalıcı yedeğe alınır.
    fn install_dir(&self, t: Target, rel: &str, files: &nuxut_boot_theme::ThemeFiles, m: &mut Manifest, tx: &mut Tx) -> Result<String> {
        let staged = self.ops.stage_dir(rel, files)?;
        tx.on_fail_remove_dir(&staged);
        let ours = matches!(m.find(rel), Some(Entry::Dir { .. }));
        let previous = swap_dir(&self.ops, tx, rel, &staged, !ours)?;
        // Yeniden uygulamada ilk kurulumda kenara alınan yabancı dizin kaydı korunur.
        let previous = match m.find(rel) {
            Some(Entry::Dir { previous, .. }) => previous.clone(),
            _ => previous,
        };
        self.record(m, Entry::Dir { target: t, path: rel.to_owned(), previous })?;
        Ok(rel.to_owned())
    }

    fn apply_grub(&self, info: &SystemInfo, opts: &ThemeOptions, fonts: Option<&(Chosen, Chosen)>, m: &mut Manifest, tx: &mut Tx) -> Result<()> {
        let g = info.grub.as_ref().ok_or_else(|| err("GRUB bulunamadı"))?;
        let (ui, mono) = fonts.ok_or_else(|| err("sistem font dizininde uygun font yok (Noto Sans, DejaVu Sans…)"))?;
        let theme_rel = format!("{}/{}", g.dir, grub::THEME_SUBDIR);
        let ev = &self.events;

        ev.step(Target::Grub, "theme", StepState::Start);
        let mut files = grub::generate(opts).map_err(|e| err(e.to_string()))?;
        self.ops.ensure_dir(&format!("{}/themes", g.dir))?;
        for spec in grub::fonts(opts.scale) {
            let src = if spec.mono { mono.path } else { ui.path };
            let (sdir, sname) = crate::safefs::split(src)?;
            if !self.ops.root.dir(sdir)?.owned_file(sname)? {
                return Err(err(format!("/{src}: font dosyası kökün değil")));
            }
            let pf2 = self.make_font(&theme_rel, &spec, src)?;
            files.push(spec.file.clone(), pf2);
        }
        self.install_dir(Target::Grub, &theme_rel, &files, m, tx)?;
        ev.step(Target::Grub, "theme", StepState::Done);

        ev.step(Target::Grub, "dropin", StepState::Start);
        let dropin_dir = grub::DROPIN_DIR.trim_start_matches('/');
        self.ops.ensure_dir(dropin_dir)?;
        let dropin = format!("{dropin_dir}/{}", grub::DROPIN_NAME);
        self.edit_file(Target::Grub, &dropin, grub::dropin(&format!("/{theme_rel}/theme.txt")).as_bytes(), m, tx)?;
        ev.step(Target::Grub, "dropin", StepState::Done);

        self.regen_grub(&g.dir, true, m, tx)
    }

    /// PF2 üretimi: grub-mkfont geçici bir dosyaya, sonra adı doğrulanıp belleğe alınır.
    fn make_font(&self, theme_rel: &str, spec: &grub::FontSpec, src: &str) -> Result<Vec<u8>> {
        let (parent, name) = crate::safefs::split(theme_rel)?;
        let work = format!("{parent}/.{name}.nuxut-new");
        // Font geçici tema dizinine yazılır (kökün, yeni kurulmuş); stage_dir ardından onu siler
        // ve tüm dosyaları yeniden yazar.
        let mut tmp = nuxut_boot_theme::ThemeFiles::default();
        tmp.push("theme.txt", Vec::new());
        self.ops.stage_dir(theme_rel, &tmp)?;
        let out = format!("{work}/{}", spec.file);
        let size = spec.size.to_string();
        let res = self.run(
            Tool::GrubMkfont,
            &["-s", &size, "-n", spec.family, "-r", PF2_RANGES, "-o", &self.ops.root.abs(&out), &self.ops.root.abs(src)],
        );
        let bytes = res.and_then(|()| self.ops.read(&out, MAX_FONT_OUT)?.ok_or_else(|| err("grub-mkfont çıktı üretmedi")));
        let _ = self.ops.remove_dir(&work);
        let bytes = bytes?;
        let font = pf2::Font::parse(bytes.clone()).map_err(|e| err(e.to_string()))?;
        if font.name != spec.grub_name() {
            return Err(err(format!("PF2 adı beklenmedik: {:?} (beklenen {:?})", font.name, spec.grub_name())));
        }
        Ok(bytes)
    }

    /// grub.cfg'yi yeniden üretir, doğrular ve yerine koyar (bkz. modül belgesi).
    fn regen_grub(&self, dir: &str, expect_theme: bool, m: &mut Manifest, tx: &mut Tx) -> Result<()> {
        let ev = &self.events;
        let cfg = format!("{dir}/grub.cfg");
        let new_rel = format!("{dir}/grub.cfg.nuxut-new");
        ev.step(Target::Grub, "grub-mkconfig", StepState::Start);
        let old = self.ops.read(&cfg, detect::MAX_GRUB_CFG)?.ok_or_else(|| err(format!("/{cfg} yok")))?;
        let old = String::from_utf8_lossy(&old).into_owned();
        if !grub::is_generated(&old) {
            return Err(err(format!("/{cfg} grub-mkconfig ile üretilmemiş; üstüne yazılmaz")));
        }
        let partial = format!("{new_rel}.new");
        self.ops.remove_file(&new_rel)?;
        self.ops.remove_file(&partial)?;
        tx.on_fail_remove_file(&new_rel);
        tx.on_fail_remove_file(&partial);
        self.run(Tool::GrubMkconfig, &["-o", &self.ops.root.abs(&new_rel)])?;
        let new = self.ops.read(&new_rel, detect::MAX_GRUB_CFG)?.ok_or_else(|| err("grub-mkconfig çıktı üretmedi"))?;
        self.run(Tool::GrubScriptCheck, &[&self.ops.root.abs(&new_rel)])?;
        let new_s = String::from_utf8_lossy(&new);
        grub::check_new_cfg(Some(&old), &new_s, expect_theme)?;
        ev.step(Target::Grub, "grub-mkconfig", StepState::Done);

        let backup = self.ops.backup(&cfg)?.ok_or_else(|| err(format!("/{cfg} yedeklenemedi")))?;
        tx.on_fail_remove_file(&backup);
        self.ops.rename(&new_rel, &cfg)?;
        tx.on_fail_rename(&backup, &cfg);
        match m.find(&cfg).cloned() {
            Some(Entry::Edit { backup: original, .. }) => {
                // Özgün yedek zaten var: bu kopya yalnız işlem için.
                tx.on_success_remove_file(&backup);
                if expect_theme {
                    self.record(m, Entry::Edit { target: Target::Grub, path: cfg, backup: original, hash: hash(&new) })?;
                } else {
                    // Geri alma: grub.cfg yeniden üretildi; özgün yedeğe artık gerek yok.
                    tx.on_success_remove_file(&original);
                }
            }
            _ if expect_theme => self.record(m, Entry::Edit { target: Target::Grub, path: cfg, backup, hash: hash(&new) })?,
            _ => tx.on_success_remove_file(&backup),
        }
        Ok(())
    }

    fn apply_limine(&self, info: &SystemInfo, opts: &ThemeOptions, m: &mut Manifest, tx: &mut Tx) -> Result<()> {
        let path = match &info.limine {
            Some(LimineConf::Ok { path, .. }) => path.clone(),
            other => return Err(err(format!("Limine yapılandırması uygun değil: {other:?}"))),
        };
        if !policy::allowed(&path) {
            return Err(OpError::Denied(path));
        }
        self.events.step(Target::Limine, "limine.conf", StepState::Start);
        let old = self.ops.read(&path, limine::MAX_CONF)?.ok_or_else(|| err(format!("/{path} yok")))?;
        let text = limine::decode(&old).map_err(|e| err(e.to_string()))?;
        let new = limine::apply_block(text, &opts.colors).map_err(|e| err(e.to_string()))?;
        if new.as_bytes() != old.as_slice() {
            self.edit_file(Target::Limine, &path, new.as_bytes(), m, tx)?;
        }
        let back = self.ops.read(&path, limine::MAX_CONF)?.unwrap_or_default();
        if back != new.as_bytes() {
            return Err(err(format!("/{path}: yazılan içerik geri okunamadı")));
        }
        self.events.step(Target::Limine, "limine.conf", StepState::Done);
        Ok(())
    }

    fn apply_plymouth(&self, info: &SystemInfo, opts: &ThemeOptions, m: &mut Manifest, tx: &mut Tx) -> Result<()> {
        let ev = &self.events;
        ev.step(Target::Plymouth, "theme", StepState::Start);
        let mut files = plymouth::generate(opts).map_err(|e| err(e.to_string()))?;
        if let Ok(Some(png)) = self.ops.read(PLYMOUTH_SPINNER_KEYMAP, MAX_KEYMAP)
            && png.starts_with(b"\x89PNG\r\n\x1a\n")
        {
            files.push("keymap-render.png", png);
        }
        self.install_dir(Target::Plymouth, PLYMOUTH_THEME, &files, m, tx)?;
        ev.step(Target::Plymouth, "theme", StepState::Done);

        ev.step(Target::Plymouth, "plymouthd.conf", StepState::Start);
        self.ops.ensure_dir("etc/plymouth")?;
        let old = self.ops.read(PLYMOUTH_CONF, detect::MAX_SMALL)?.unwrap_or_default();
        let new = plymouth::set_daemon_theme(&String::from_utf8_lossy(&old), plymouth::THEME_NAME);
        self.edit_file(Target::Plymouth, PLYMOUTH_CONF, new.as_bytes(), m, tx)?;
        ev.step(Target::Plymouth, "plymouthd.conf", StepState::Done);

        self.rebuild_initramfs(info, tx)
    }

    /// Plymouth initramfs'teyse imgeleri yeniden kurar (yedekli; bkz. modül belgesi).
    fn rebuild_initramfs(&self, info: &SystemInfo, tx: &mut Tx) -> Result<()> {
        let images = match &info.plymouth.initramfs {
            Initramfs::Mkinitcpio { hook: true, images: Some(images) } => images,
            Initramfs::Mkinitcpio { hook: true, images: None } | Initramfs::Dracut => {
                self.events.warn(Some(Target::Plymouth), "initramfs elle yeniden kurulmalı");
                return Ok(());
            }
            _ => return Ok(()),
        };
        let ev = &self.events;
        ev.step(Target::Plymouth, "mkinitcpio", StepState::Start);
        let rels: Vec<String> = images.iter().map(|i| i.trim_start_matches('/').to_owned()).collect();
        if let Some(bad) = rels.iter().find(|r| !policy::initramfs_image(r)) {
            return Err(OpError::Denied(bad.clone()));
        }
        // Yer denetimi: dizin başına mevcut imgelerin toplamı + pay.
        let mut per_dir: Vec<(String, u64)> = Vec::new();
        for r in &rels {
            let size = self.ops.size(r).unwrap_or(None).unwrap_or(0);
            let dir = r.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
            match per_dir.iter_mut().find(|(d, _)| *d == dir) {
                Some((_, s)) => *s += size,
                None => per_dir.push((dir, size)),
            }
        }
        for (dir, need) in &per_dir {
            let free = self.ops.free_bytes(dir)?;
            if free < need + SPACE_MARGIN {
                return Err(err(format!("/{dir}: initramfs yedeği için yer yok ({} MiB gerekli, {} MiB boş)", (need + SPACE_MARGIN) >> 20, free >> 20)));
            }
        }
        for r in &rels {
            if let Some(b) = self.ops.backup(r)? {
                tx.on_fail_remove_file(&b);
                tx.on_fail_rename(&b, r);
                tx.on_success_remove_file(&b);
            }
        }
        self.run(Tool::Mkinitcpio, &["-P"])?;
        ev.step(Target::Plymouth, "mkinitcpio", StepState::Done);
        Ok(())
    }

    fn revert_target(&self, t: Target, entries: &[Entry], info: &SystemInfo, tx: &mut Tx) -> Result<()> {
        self.events.step(t, "restore", StepState::Start);
        let mut grub_dir = None;
        for e in entries.iter().rev() {
            match e {
                Entry::Dir { path, previous, .. } => {
                    let (parent, name) = crate::safefs::split(path)?;
                    let old = format!("{parent}/.{name}.nuxut-old");
                    self.ops.remove_dir(&old)?;
                    if self.ops.exists(path) {
                        self.ops.rename(path, &old)?;
                        tx.on_fail_rename(&old, path);
                        tx.on_success_remove_dir(&old);
                    }
                    if let Some(p) = previous
                        && self.ops.exists(p)
                    {
                        self.ops.rename(p, path)?;
                        tx.on_fail_rename(path, p);
                    }
                }
                Entry::File { path, hash: h, .. } => match self.ops.read(path, detect::MAX_SMALL)? {
                    Some(cur) if hash(&cur) == *h => {
                        tx.on_fail_write(path, cur);
                        self.ops.remove_file(path)?;
                    }
                    Some(_) => self.events.warn(Some(t), format!("/{path} Nuxut'tan sonra değişmiş; bırakıldı")),
                    None => {}
                },
                Entry::Edit { path, .. } if path.ends_with("/grub.cfg") => {
                    grub_dir = path.strip_suffix("/grub.cfg").map(str::to_owned);
                }
                Entry::Edit { path, backup, hash: h, .. } => {
                    let cur = self.ops.read(path, detect::MAX_GRUB_CFG)?;
                    let orig = self.ops.read(backup, detect::MAX_GRUB_CFG)?;
                    let restored = match (&cur, orig) {
                        (Some(c), Some(o)) if hash(c) == *h => Some(o),
                        (Some(c), orig) => structural_undo(path, &String::from_utf8_lossy(c), orig.as_deref()),
                        (None, orig) => orig,
                    };
                    match restored {
                        Some(bytes) => {
                            match cur {
                                Some(c) => tx.on_fail_write(path, c),
                                None => tx.on_fail_remove_file(path),
                            }
                            self.ops.write(path, &bytes)?;
                            tx.on_success_remove_file(backup);
                        }
                        None => self.events.warn(Some(t), format!("/{path} geri alınamadı; yedek: /{backup}")),
                    }
                }
            }
        }
        self.events.step(t, "restore", StepState::Done);
        match t {
            Target::Grub => {
                let dir = grub_dir.or_else(|| info.grub.as_ref().map(|g| g.dir.clone()));
                match dir {
                    Some(dir) => {
                        let mut m = self.load_manifest()?;
                        self.regen_grub(&dir, false, &mut m, tx)
                    }
                    None => Ok(()),
                }
            }
            Target::Plymouth => self.rebuild_initramfs(info, tx),
            Target::Limine => Ok(()),
        }
    }
}

/// İçerik Nuxut'tan sonra başkasınca değiştirilmişse yalnız bizim parçamızı geri alır.
fn structural_undo(path: &str, cur: &str, orig: Option<&[u8]>) -> Option<Vec<u8>> {
    if path == PLYMOUTH_CONF {
        let previous = orig.and_then(|o| plymouth::daemon_theme(&String::from_utf8_lossy(o)));
        let out = match previous {
            Some(theme) => plymouth::set_daemon_theme(cur, &theme),
            None => {
                let mut s: String = cur.lines().filter(|l| l.trim() != format!("Theme={}", plymouth::THEME_NAME)).collect::<Vec<_>>().join("\n");
                s.push('\n');
                s
            }
        };
        return Some(out.into_bytes());
    }
    if path.ends_with("limine.conf") {
        return limine::remove_block(cur).ok().map(String::into_bytes);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_undo_keeps_foreign_changes() {
        let cur = "[Daemon]\nTheme=nuxut\nShowDelay=5\n";
        assert_eq!(structural_undo(PLYMOUTH_CONF, cur, Some(b"[Daemon]\nTheme=bgrt\n")).unwrap(), b"[Daemon]\nTheme=bgrt\nShowDelay=5\n");
        assert_eq!(structural_undo(PLYMOUTH_CONF, cur, None).unwrap(), b"[Daemon]\nShowDelay=5\n");
        let conf = limine::apply_block("/Arch\n    protocol: linux\n", &BootColors::fallback()).unwrap() + "/Yeni\n    protocol: linux\n";
        assert_eq!(structural_undo("boot/limine.conf", &conf, None).unwrap(), b"/Arch\n    protocol: linux\n/Yeni\n    protocol: linux\n");
        assert_eq!(structural_undo("etc/x", "", None), None);
    }
}

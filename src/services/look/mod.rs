//! Birleşik görünüm (ADR-0004): framework teması + `framework.toml [desktop]` seçimleri →
//! GTK3/GTK4-libadwaita, Qt (KDE platform teması; yedek qt6ct), kdeglobals, ikon, imleç, yazı
//! tipi ve oturum ortamı hedefleri.
//!
//! Katmanlar:
//! - saf üreticiler: [`gtk`], [`qt`], [`fonts`], [`env`], [`palette`] (rol kümesi `nuxut-palette`),
//! - seçim ve sistem yoklaması: [`selection`],
//! - plan (ayar başına parçalar + canlı eylemler): [`plan`],
//! - uygulama kapısı: [`backend`] (trait + rapor; [`backend::Disabled`] hiçbir şey yazmaz) ve
//!   gerçek arka uç [`writer`] (`nuxut-apply` + canlı eylemler; test örneği kapısı),
//! - `msg look status|plan` yanıtları: [`status`].
//! - aynı planın yeniden uygulanmaması ve yeniden yazım döngüsü bekçisi: [`guard`].
//! - kurulu tema dizinleri izleyicisi (`themes_rev`): [`watch`].
//!
//! Çalışma anı (shell `app/look.rs`, `nuxut-look` thread'i): tema / `framework.toml` değişince
//! [`sync`] — birleşik görünüm açıksa plan uygulanır (her şey aynıysa `nuxut-apply` `NoOp`, canlı
//! eylem yok), kapalıysa ve Nuxut'un yazdıkları diskteyse geri yüklenir, değilse hiçbir şey.
//!
//! Buradaki IO yalnızca okuma ve küçük: `framework.toml` / `palette.toml` (sınırlı),
//! `environment.d` taraması, tema dizinlerinde var/yok yoklaması.

pub mod app_refresh;
pub mod backend;
pub mod catalog;
pub mod env;
pub mod folders;
pub mod fonts;
pub mod gtk;
pub mod gtk_theme;
pub mod gtk_watch;
pub mod kitty;
pub mod guard;
pub mod oneshot;
pub mod palette;
pub mod plan;
pub mod qt;
pub mod selection;
pub mod status;
pub mod watch;
pub mod writer;

use nuxut::theme::{FrameworkConfig, GeneratedPalette, PaletteMeta};

use self::backend::ApplyReport;
use self::plan::{Dirs, LookInput, LookPlan};
use self::selection::{FrameworkLook, FsProbe, Resolved};

/// `framework.toml`'dan görünüm seçimleri (framework şeması ve doğrulaması; okuma
/// `nuxut_config::load` ile sınırlı). Dosya yoksa varsayılanlar (birleşik görünüm açık, framework
/// 604a790); okunamaz/bozuksa kapalı + uyarı.
pub fn read_framework() -> FrameworkLook {
    let Some(path) = nuxut_config::config_path(nuxut::theme::CONFIG_NAME) else { return FrameworkLook::default() };
    match nuxut_config::load::<FrameworkConfig>(&path) {
        Ok(l) if l.from_file => FrameworkLook::from_config(&l.value, l.warnings),
        Ok(_) => FrameworkLook::default(),
        Err(_) => selection::broken(),
    }
}

/// Duvar kağıdı paletinin bilgisi (tohum, şema, kontrast); yoksa/bozuksa `None`.
pub fn palette_meta() -> Option<PaletteMeta> {
    let path = nuxut::theme::palette_path()?;
    GeneratedPalette::load(&path).ok().filter(|l| l.from_file).map(|l| l.value.meta)
}

/// O anki görünüm durumu (okuma + saf hesap).
#[derive(Debug, Clone)]
pub struct LookState {
    pub dirs: Dirs,
    pub framework: FrameworkLook,
    pub resolved: Resolved,
    pub plan: LookPlan,
    /// Kurulu ikon/imleç temaları (framework tarayıcısı).
    pub icons: Vec<selection::IconEntry>,
    pub cursors: Vec<selection::CursorEntry>,
    /// qt6ct yolunda metin ölçeği (`QT_FONT_DPI`); çakışma taraması planla aynı değeri bilir.
    pub qt_font_dpi: Option<u32>,
}

impl LookState {
    /// `theme`: runtime'ın çözülmüş teması (renkler + yazı tipleri).
    pub fn read(theme: &nuxut::Theme) -> Result<Self, String> {
        let dirs = Dirs::from_env().ok_or("HOME tanımlı değil")?;
        let framework = read_framework();
        let probe = FsProbe::from_env(Some(&dirs.home), Some(&dirs.data));
        let meta = if framework.wallpaper { palette_meta() } else { None };
        Ok(Self::compute(theme, framework, meta.as_ref(), dirs, &probe))
    }

    /// Saf kısım: kurulu olana göre çözümler, planı üretir. Kip framework'ün çözdüğü
    /// `theme.mode`'dur (`[colors] mode`; `auto`: `palette.toml` kipi → yüzey parlaklığı → koyu,
    /// `ColorsConfig::resolve_mode`): GTK/Qt, Nuxut'un o an çizdiği kiple aynıdır. `meta` yalnızca
    /// türetmenin tohumu içindir.
    pub fn compute(
        theme: &nuxut::Theme,
        mut framework: FrameworkLook,
        meta: Option<&PaletteMeta>,
        dirs: Dirs,
        probe: &dyn selection::SystemProbe,
    ) -> Self {
        let mode = nuxut_palette::Mode::from(theme.mode);
        if !palette::tokens_apply(&theme.colors, mode) {
            framework.warnings.push(format!(
                "colors.surface {} kipiyle uyuşmuyor; Nuxut renkleri uygulamalara aktarılmadı, türetilmiş palet kullanıldı",
                mode.name()
            ));
        }
        let resolved = selection::resolve(&framework.desktop, mode, probe);
        let gtk_base = match (&resolved.gtk_base_dir, framework.desktop.unified) {
            (Some(dir), true) => probe.gtk_base(&resolved.gtk_theme, dir),
            _ => None,
        };
        let css_links: Vec<std::path::PathBuf> = if framework.desktop.unified {
            ["gtk-4.0/gtk.css", "gtk-4.0/gtk-dark.css", "gtk-3.0/gtk.css"]
                .into_iter()
                .map(|rel| dirs.config.join(rel))
                .filter(|p| probe.foreign_link(p))
                .collect()
        } else {
            Vec::new()
        };
        let folder_set = if framework.desktop.unified && folders::supports(&resolved.icon_theme) {
            probe.folder_icons(&resolved.icon_theme)
        } else {
            None
        };
        let plan = plan::build(
            &LookInput {
                colors: &theme.colors,
                generated: meta,
                color_theme: theme.color_theme,
                typography: &theme.typography,
                shape: &theme.shape,
                contrast: theme.contrast,
                reduced_motion: theme.reduced_motion(),
                selection: &framework.desktop,
                resolved: &resolved,
                gtk_base: gtk_base.as_ref(),
                css_links: &css_links,
                folders: folder_set.as_ref(),
            },
            &dirs,
        );
        let qt_font_dpi = fonts::qt_font_dpi(&theme.typography, resolved.qt_backend == Some(selection::QtBackend::Qt6ct));
        Self { dirs, framework, resolved, plan, icons: probe.icon_themes(), cursors: probe.cursor_themes(), qt_font_dpi }
    }

    /// Çakışmalar: shell'in süreç ortamı + kullanıcının `environment.d` dosyaları.
    fn conflicts(&self) -> Vec<env::Conflict> {
        let ours = env::variables(&self.resolved, self.qt_font_dpi);
        let session: Vec<(String, String)> = std::env::vars().filter(|(k, _)| env::WATCHED.contains(&k.as_str())).collect();
        let mut out = env::scan_session(session.iter().map(|(k, v)| (k.as_str(), v.as_str())), &ours);
        out.extend(env::scan_env_dir(&self.dirs.config.join("environment.d"), &ours));
        out
    }

    /// `writer`: etkin arka ucun adı; `last_apply`: görünüm thread'inin son raporu; `themes_rev`:
    /// [`watch::ThemeWatcher::rev`].
    pub fn status_json(&self, writer: &'static str, last_apply: Option<ApplyReport>, themes_rev: u64) -> String {
        let s = status::status(status::StatusInput {
            framework: &self.framework,
            resolved: &self.resolved,
            icons: &self.icons,
            cursors: &self.cursors,
            conflicts: self.conflicts(),
            plan: &self.plan,
            writer,
            last_apply,
            themes_rev,
        });
        serde_json::to_string(&s).unwrap_or_else(|e| format!("error: {e}"))
    }

    pub fn plan_json(&self, writer: &'static str) -> String {
        let entries = plan::summarize(&self.plan, &self.dirs);
        let reply = status::plan_reply(self.framework.desktop.unified, &self.plan, entries, writer);
        serde_json::to_string(&reply).unwrap_or_else(|e| format!("error: {e}"))
    }

    /// `hypr_config` üreticisinin `env` girdisi (§5.4): yalnızca imleç değişkenleri; birleşik görünüm
    /// kapalıyken boş.
    pub fn cursor_env(&self) -> Vec<(String, String)> {
        if !self.framework.desktop.unified {
            return Vec::new();
        }
        env::cursor_variables(&self.resolved).into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
    }
}

/// Bir eşitleme turunun sonucu (görünüm thread'i → ana thread).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncResult {
    /// Bir şey denendiyse raporu (`None`: kapalı ve etkin değil, ya da okuma hatası).
    pub report: Option<ApplyReport>,
    pub cursor_env: Vec<(String, String)>,
    /// Doğrulama turunda (GTK dosyaları dışarıdan değişmişti) yazım oldu: başka bir araç Nuxut
    /// dosyalarını değiştirmişti, Nuxut yeniden uyguladı (`look::gtk_watch`).
    pub repaired: bool,
}

/// Bir tur: oku → (açıksa) planı uygula / (kapalı ve etkinse) geri yükle. Bloklayan IO; yalnızca
/// görünüm thread'inde. `state` dışarıdan verilir (testler geçici `Dirs` ile kurar).
pub fn sync(state: &LookState, env: &nuxut_apply::Env, sink: &mut dyn writer::LiveSink) -> SyncResult {
    sync_with(state, env, sink, &mut || false, &[])
}

/// [`sync`], yarıda bırakılabilir ([`backend::apply_with`]): `cancel` daha yeni istek gelince
/// doğru döner; `carried` yarıda kalan önceki turların yazdığı ayarlar (canlı eylemleri burada).
/// Kapatma (geri yükleme) yarıda bırakılmaz.
pub fn sync_with(
    state: &LookState,
    env: &nuxut_apply::Env,
    sink: &mut dyn writer::LiveSink,
    cancel: &mut dyn FnMut() -> bool,
    carried: &[&'static str],
) -> SyncResult {
    let report = if state.framework.desktop.unified {
        let mut backend = writer::ApplyBackend::new(env, sink);
        Some(backend::apply_with(&state.plan, &mut backend, cancel, carried))
    } else {
        writer::deactivate(env, sink, &state.dirs)
    };
    SyncResult { report, cursor_env: state.cursor_env(), repaired: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> Dirs {
        Dirs { home: "/home/u".into(), config: "/home/u/.config".into(), data: "/home/u/.local/share".into() }
    }

    fn mode_of(state: &LookState) -> &'static str {
        let s = status::status(status::StatusInput {
            framework: &state.framework,
            resolved: &state.resolved,
            icons: &state.icons,
            cursors: &state.cursors,
            conflicts: Vec::new(),
            plan: &state.plan,
            writer: "disabled",
            last_apply: None,
            themes_rev: 0,
        });
        s.mode
    }

    /// Framework'ün `framework.toml` (+ palet) ile çözdüğü tema ve shell'in seçimleri.
    fn framework(text: &str, generated: Option<&GeneratedPalette>) -> (nuxut::Theme, FrameworkLook) {
        let (cfg, _) = nuxut_config::parse_str::<FrameworkConfig>(text).unwrap();
        (nuxut::Theme::from_generated(&cfg, generated).0, selection::parse_framework(text))
    }

    fn gtk3_of(state: &LookState) -> String {
        state.plan.changes[0]
            .writes
            .iter()
            .find(|(f, _)| matches!(f, plan::Fragment::ManagedFile { file } if file.ends_with("gtk-3.0/nuxut.css")))
            .map(|(_, text)| text.clone())
            .unwrap()
    }

    /// Kip framework'ün `Theme::mode`'u ile aynı: auto (varsayılan) → palette.toml kipi → yüzey
    /// parlaklığı → koyu; sabit kip kazanır.
    #[test]
    fn mode_is_framework_resolved_mode() {
        let probe = selection::tests::FakeProbe::full();
        let light_meta = GeneratedPalette {
            meta: PaletteMeta { mode: Some("light".into()), ..Default::default() },
            ..Default::default()
        };
        let cases: [(&str, Option<&GeneratedPalette>, &str); 6] = [
            ("[desktop]\nunified = true\n", None, "dark"),
            ("[colors]\nsurface = \"#f8f9ff\"\n[desktop]\nunified = true\n", None, "light"),
            ("[colors]\nsource = \"wallpaper\"\n[desktop]\nunified = true\n", Some(&light_meta), "light"),
            // Palet yalnızca source = wallpaper iken sayılır.
            ("[desktop]\nunified = true\n", Some(&light_meta), "dark"),
            ("[colors]\nsource = \"wallpaper\"\nmode = \"dark\"\n[desktop]\nunified = true\n", Some(&light_meta), "dark"),
            ("[colors]\nmode = \"light\"\n[desktop]\nunified = true\n", None, "light"),
        ];
        for (text, generated, want) in cases {
            let (theme, fw) = framework(text, generated);
            let state = LookState::compute(&theme, fw, generated.map(|g| &g.meta), dirs(), &probe);
            assert_eq!(mode_of(&state), theme.mode.name(), "{text}");
            assert_eq!(mode_of(&state), want, "{text}");
            assert!(state.framework.warnings.is_empty(), "{text}: {:?}", state.framework.warnings);
            assert!(gtk3_of(&state).contains(&format!("GTK 3 ({want})")), "{text}");
        }
    }

    #[test]
    fn light_surface_resolves_light_everywhere() {
        let (theme, fw) = framework("[colors]\nsurface = \"#f8f9ff\"\n[desktop]\nunified = true\n", None);
        let state = LookState::compute(&theme, fw, None, dirs(), &selection::tests::FakeProbe::full());
        assert_eq!(state.resolved.icon_theme, "Papirus-Light");
        assert!(gtk3_of(&state).contains("@define-color window_bg_color #f8f9ff;"));
    }

    /// Sabit kiple çelişen açık yüzey: framework kipi kazanır, token'lar aktarılmaz, uyarılır.
    #[test]
    fn explicit_surface_against_fixed_mode_warns() {
        let (theme, fw) = framework("[colors]\nmode = \"dark\"\nsurface = \"#f8f9ff\"\n[desktop]\nunified = true\n", None);
        let state = LookState::compute(&theme, fw, None, dirs(), &selection::tests::FakeProbe::full());
        assert_eq!(mode_of(&state), "dark");
        assert_eq!(state.framework.warnings.len(), 1, "{:?}", state.framework.warnings);
        assert!(!gtk3_of(&state).contains("#f8f9ff"));
    }

    // -----------------------------------------------------------------------------------------
    // Yalıtılmış uçtan uca: nuxut-apply test ortamı (geçici kök, test örneği; dconf/systemd/
    // Hyprland'e hiçbir yol yok), kayıtçı canlı gönderici.

    #[derive(Debug, Default)]
    struct Rec(Vec<&'static str>);

    impl writer::LiveSink for Rec {
        fn run(&mut self, a: &plan::LiveAction) -> Result<(), String> {
            self.0.push(a.name());
            Ok(())
        }
    }

    /// Bütün hedefler test kökünün `config` dizini altında (kapı yalnızca orayı ve `state`'i kabul eder).
    fn temp_dirs(root: &std::path::Path) -> Dirs {
        let home = root.join("config/home");
        Dirs { config: root.join("config"), data: home.join(".local/share"), home }
    }

    fn state_for(text: &str, dirs: Dirs) -> LookState {
        let (theme, fw) = framework(text, None);
        LookState::compute(&theme, fw, None, dirs, &selection::tests::FakeProbe::full())
    }

    fn snapshots(env: &nuxut_apply::Env) -> Vec<nuxut_apply::SnapshotSummary> {
        nuxut_apply::Applier::open(env.clone()).list(&nuxut_apply::Filter { setting: None, limit: 200 }).unwrap_or_default()
    }

    fn read(p: &std::path::Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }

    /// Varsayılan (kde yolu) planın ayar sayısı (qt_backend parçası yok).
    fn setting_count() -> usize {
        plan::setting::ALL.len() - 1
    }

    const ON: &str = "[desktop]\nunified = true\n";
    /// Açıkça kapalı (framework 604a790: anahtar ya da dosya yoksa açık).
    const OFF: &str = "[desktop]\nunified = false\n";

    /// Kullanıcının kdeglobals'ı (Nuxut öncesi).
    const USER_KDE: &str = "[General]\nMyKey=1\n";

    /// Kapatma sonrası kdeglobals byte-eşit kullanıcının ilk hâli: Nuxut'un yarattığı bölümler
    /// (`section_created`) boş kalınca başlıklarıyla silinir.
    fn assert_user_kdeglobals(cfg: &std::path::Path) {
        assert_eq!(read(&cfg.join("kdeglobals")).unwrap(), USER_KDE);
    }

    #[test]
    fn apply_writes_snapshots_noops_and_restores_in_temp_root() {
        use plan::setting::{CURSOR, FONTS, ICONS, MODE, MOTION, UNIFIED};
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let cfg = dirs.config.clone();
        // Kullanıcının önceden var olan dosyaları: yalnızca yönetilen anahtar/satır değişmeli.
        std::fs::create_dir_all(cfg.join("gtk-3.0")).unwrap();
        let user_css = "/* benim */\nwindow { margin: 1px; }\n";
        let user_ini = "[Settings]\ngtk-cursor-theme-name=Adwaita\ngtk-cursor-theme-size=32\ngtk-enable-animations=0\n";
        std::fs::write(cfg.join("gtk-3.0/gtk.css"), user_css).unwrap();
        std::fs::write(cfg.join("gtk-3.0/settings.ini"), user_ini).unwrap();
        std::fs::write(cfg.join("kdeglobals"), USER_KDE).unwrap();

        // look.mode tek işlemde ~105 parça (kdeglobals renkleri 7×12): bölünmeden yazılır, tek
        // snapshot (nuxut-apply MAX_FRAGMENTS 256 + adres bütçesi).
        let on_state = state_for(ON, dirs.clone());
        let mode_frags = on_state.plan.changes.iter().find(|c| c.setting == MODE).unwrap().writes.len();
        assert!(mode_frags > 64, "eski 64 sınırını aşan tam look.mode sınanmalı: {mode_frags}");
        let applied: Vec<&str> = vec![MODE, ICONS, CURSOR, FONTS, MOTION, UNIFIED];

        // Kapalı + hiç uygulanmamış: hiçbir şey yazılmaz, depo bile açılmaz.
        let mut rec = Rec::default();
        let off = sync(&state_for(OFF, dirs.clone()), &env, &mut rec);
        assert_eq!(off, SyncResult::default());
        assert!(snapshots(&env).is_empty());
        assert!(!cfg.join("gtk-4.0").exists());

        // Açık: her ayar yazılır, ayar başına tek snapshot; canlı eylem yok (test örneği).
        let r = sync(&on_state, &env, &mut rec).report.unwrap();
        assert!(r.ok && r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.written, applied);
        assert!(r.live.is_empty() && rec.0.is_empty(), "test örneği canlı eylem göndermez: {:?}", rec.0);
        let snaps = snapshots(&env);
        assert_eq!(snaps.len(), applied.len(), "{snaps:?}");
        for s in &applied {
            assert_eq!(snaps.iter().filter(|x| x.setting.as_str() == *s).count(), 1, "{s}");
        }
        assert!(snaps.iter().all(|s| s.pinned));
        let ini = read(&cfg.join("gtk-3.0/settings.ini")).unwrap();
        assert!(ini.contains("gtk-cursor-theme-name=Vimix-cursors") && ini.contains("gtk-enable-animations=1"), "hareket açık: kullanıcının 0'ı ezilir (look.motion) {ini}");
        assert!(ini.contains("gtk-xft-dpi=98304") && !ini.contains("gtk-enable-animations=0"), "{ini}");
        let kde = read(&cfg.join("kdeglobals")).unwrap();
        assert!(kde.contains("MyKey=1") && kde.contains("[Icons]"), "{kde}");
        assert!(read(&dirs.home.join(".icons/default/index.theme")).unwrap().contains("Inherits=Vimix-cursors"));
        let envd = read(&cfg.join("environment.d/99-nuxut-look.conf")).unwrap();
        assert!(envd.starts_with("# Nuxut — "));
        assert!(envd.contains("XCURSOR_THEME=Vimix-cursors\n") && envd.contains("QT_QPA_PLATFORMTHEME=kde\n"));
        assert!(read(&cfg.join("gtk-4.0/nuxut.css")).unwrap().starts_with("/* Nuxut — "));
        assert!(read(&cfg.join("gtk-4.0/gtk.css")).unwrap().contains("@import url(\"nuxut.css\");"), "dosya yokken yaratılır");
        let gtk3 = read(&cfg.join("gtk-3.0/gtk.css")).unwrap();
        assert!(gtk3.starts_with(user_css) && gtk3.contains("@import url(\"nuxut.css\");"), "{gtk3}");
        for group in ["[Colors:Window]", "[Colors:View]", "[Colors:Button]", "[Colors:Selection]", "[Colors:Tooltip]", "[Colors:Complementary]", "[Colors:Header]"] {
            assert!(kde.contains(group), "look.mode tek işlemde eksiksiz: {group}");
        }
        assert!(read(&dirs.data.join("color-schemes/Nuxut.colors")).is_some());
        // look.mode tek işlem = tek snapshot ve parçaların hepsi onda.
        let mode_snap = snaps.iter().find(|s| s.setting.as_str() == MODE).unwrap();
        let mode_change = on_state.plan.changes.iter().find(|c| c.setting == MODE).unwrap();
        let mode_req = writer::to_change(mode_change, false).unwrap();
        assert_eq!(mode_snap.fragments, mode_req.writes.len(), "test örneğinde yalnız dconf düşülür: {mode_snap:?}");
        // Düşülen dconf anahtarları: color-scheme, gtk-theme, accent-color.
        assert!(mode_frags - mode_req.writes.len() <= 3, "{mode_frags}");

        // Aynı girdi: hepsi NoOp, yeni snapshot yok, dosya değişmez.
        let before = read(&cfg.join("kdeglobals"));
        let again = sync(&state_for(ON, dirs.clone()), &env, &mut rec);
        assert_eq!(again.cursor_env[0], ("XCURSOR_THEME".into(), "Vimix-cursors".into()));
        let again = again.report.unwrap();
        assert!(again.written.is_empty() && again.unchanged.len() == applied.len(), "{again:?}");
        assert_eq!(snapshots(&env).len(), applied.len());
        assert_eq!(read(&cfg.join("kdeglobals")), before);

        // Kip değişimi: yalnızca etkilenen ayarlar yazılır (Papirus kipe göre değişir).
        let light = sync(&state_for("[colors]\nmode = \"light\"\n[desktop]\nunified = true\n", dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(light.ok && light.written.contains(&ICONS) && light.written.contains(&MODE), "{light:?}");
        assert!(!light.written.contains(&CURSOR) && !light.written.contains(&FONTS) && !light.written.contains(&MOTION));
        // Türetilmiş yazım (apply_derived): araya kimse girmediğinden önceki işleme birleşir — yeni
        // snapshot oluşmaz, ilk "önce" korunur.
        assert_eq!(snapshots(&env).len(), applied.len());

        // Kapatma = Nuxut öncesine dönüş: kullanıcının dosyaları ilk hâline, Nuxut'un dosyaları silinir.
        let res = sync(&state_for(OFF, dirs.clone()), &env, &mut rec);
        let back = res.report.unwrap();
        assert!(back.ok, "{:?}", back.errors);
        assert_eq!(back.written, applied, "{back:?}");
        assert!(res.cursor_env.is_empty());
        assert_eq!(read(&cfg.join("gtk-3.0/gtk.css")).unwrap(), user_css);
        assert_eq!(read(&cfg.join("gtk-3.0/settings.ini")).unwrap(), user_ini);
        assert_user_kdeglobals(&cfg);
        for gone in [
            cfg.join("gtk-4.0/nuxut.css"),
            cfg.join("gtk-3.0/nuxut.css"),
            cfg.join("environment.d/99-nuxut-look.conf"),
            dirs.data.join("color-schemes/Nuxut.colors"),
            cfg.join("gtk-4.0/gtk.css"),
            cfg.join("gtk-4.0/settings.ini"),
            dirs.home.join(".icons/default/index.theme"),
        ] {
            assert!(!gone.exists(), "{}", gone.display());
        }
        assert!(rec.0.is_empty());
        // Artık etkin değil: kapalı turlar hiçbir şey yapmaz.
        let n = snapshots(&env).len();
        assert_eq!(sync(&state_for(OFF, dirs.clone()), &env, &mut rec), SyncResult::default());
        assert_eq!(snapshots(&env).len(), n);

        // Yeniden açıp kapatma: yine Nuxut öncesine.
        let r = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert_eq!(r.written, applied);
        assert!(read(&cfg.join("environment.d/99-nuxut-look.conf")).is_some());
        assert!(sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap().ok);
        assert_user_kdeglobals(&cfg);
        assert_eq!(read(&cfg.join("gtk-3.0/settings.ini")).unwrap(), user_ini);
        assert!(!cfg.join("environment.d/99-nuxut-look.conf").exists());
    }

    /// look.mode'un bütün parçaları (KDE renk grupları, GTK satırı — dosya yokken yaratılır —,
    /// yönetilen dosyalar) nuxut-apply adaptörlerince kabul edilip yazılır ve geri yüklenir.
    /// Üründeki gibi TEK işlemde (bölünmeden) uygulanır ve tek geri yüklemeyle döner.
    #[test]
    fn mode_fragments_are_accepted_by_adapters() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        std::fs::create_dir_all(&dirs.config).unwrap();
        std::fs::write(dirs.config.join("kdeglobals"), USER_KDE).unwrap();
        let state = state_for(ON, dirs.clone());
        let mode = state.plan.changes.iter().find(|c| c.setting == plan::setting::MODE).unwrap();
        let full = writer::to_change(mode, false).unwrap();
        assert!(full.writes.len() > 64 && writer::fits_one_transaction(&full), "{}", full.writes.len());
        let app = nuxut_apply::Applier::open(env.clone());
        let id = match app.apply(full.clone()) {
            nuxut_apply::Outcome::Applied { id } => id,
            other => panic!("{other:?}"),
        };
        let kde = read(&dirs.config.join("kdeglobals")).unwrap();
        for s in ["[Colors:Window]", "[Colors:Header]", "[ColorEffects:Inactive]", "[KDE]", "MyKey=1"] {
            assert!(kde.contains(s), "{s}: {kde}");
        }
        assert!(read(&dirs.config.join("gtk-4.0/gtk.css")).unwrap().contains("@import url(\"nuxut.css\");"));
        assert!(matches!(app.restore(&id, nuxut_apply::RestoreMode::Safe), nuxut_apply::RestoreOutcome::Restored { id: Some(_) }));
        // Byte-eşit geri dönüş (Nuxut.un yarattığı bölümler başlıklarıyla silinir).
        assert_user_kdeglobals(&dirs.config);
        assert!(!dirs.config.join("gtk-4.0/nuxut.css").exists());
        assert!(!dirs.config.join("gtk-4.0/gtk.css").exists(), "yaratılan gtk.css geri yüklemede silinir");
    }

    /// Göç: önceki sürüm `90-nuxut-look.conf`'u look.unified işlemiyle yazmıştı. Sonraki tur 90-'ı
    /// kaldırıp 99-'u yazar (snapshot uyumlu); kapatınca ikisi de yok.
    #[test]
    fn legacy_env_file_migrates_to_99() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let envd = dirs.config.join("environment.d");
        let legacy = envd.join(env::LEGACY_ENV_FILE);
        let new = envd.join(env::ENV_FILE);
        // Eski sürümün durumu: 90- dosyası ve onun (sabitlenmiş) look.unified işlemi.
        let old = nuxut_apply::Change::new(nuxut_apply::SettingId::new(plan::setting::UNIFIED).unwrap(), "eski", nuxut_apply::Source::Shell)
            .write(
                nuxut_apply::Fragment::ManagedFile { file: legacy.clone() },
                nuxut_apply::FragValue::File { bytes: env::environment_d(&[("XCURSOR_SIZE", "24".into())]).into_bytes(), mode: 0 },
            );
        let app = nuxut_apply::Applier::open(env.clone()).with_source(nuxut_apply::Source::Shell);
        assert!(matches!(app.apply(old), nuxut_apply::Outcome::Applied { .. }));
        assert!(legacy.exists());

        let mut rec = Rec::default();
        let r = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(r.ok && r.written.contains(&plan::setting::UNIFIED), "{r:?}");
        assert!(!legacy.exists(), "90- kaldırıldı");
        assert!(read(&new).unwrap().starts_with("# Nuxut — "), "99- yazıldı");
        // Göç bir kez: aynı girdi NoOp.
        let again = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(again.written.is_empty(), "{again:?}");

        let back = sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(back.ok, "{:?}", back.errors);
        assert!(!legacy.exists() && !new.exists(), "kapatınca ikisi de yok");
    }

    /// Kullanıcının elle yazdığı (başlıksız) aynı adlı dosyaya dokunulmaz.
    #[test]
    fn foreign_90_env_file_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let legacy = dirs.config.join("environment.d").join(env::LEGACY_ENV_FILE);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "FOO=1\n").unwrap();
        let mut rec = Rec::default();
        assert!(sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap().ok);
        assert!(sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap().ok);
        assert_eq!(read(&legacy).unwrap(), "FOO=1\n");
    }

    /// Önceki sürümün başlıklı yönetilen dosya olarak yarattığı `gtk.css` (kullanıcı altına kural
    /// eklemiş olabilir) artık olağan satır parçası: açma ona dokunmaz (satır zaten var), kapatma
    /// dosyayı silmez — byte-eşit kalır.
    #[test]
    fn legacy_headered_gtk_css_is_a_plain_line() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let css = dirs.config.join("gtk-4.0/gtk.css");
        std::fs::create_dir_all(css.parent().unwrap()).unwrap();
        let legacy = format!(
            "/* {}birleşik görünüm içe aktarımı */\n@import url(\"nuxut.css\");\nwindow {{ margin: 2px; }}\n",
            nuxut_apply::edit::managed::HEADER_MARK
        );
        std::fs::write(&css, &legacy).unwrap();
        let mut rec = Rec::default();
        let r = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(r.ok, "{:?}", r.errors);
        assert_eq!(read(&css).unwrap(), legacy);
        let back = sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(back.ok, "{:?}", back.errors);
        assert_eq!(read(&css).unwrap(), legacy, "kapatma eski başlıklı dosyayı silmez");
    }

    /// Test örneği kullanıcının gerçek dizinlerine asla yazamaz: hedefler "gerçek" ev dizinindeyse
    /// her ayar `skipped(test_instance)`, hiçbir dosya ve snapshot oluşmaz. Kullanıcının gerçek ev
    /// dizinine hiç dokunmamak için kapının aynı kuralı geçici bir "gerçek ev" ile sınanır
    /// (`real_home` kapının güvendiği alan; test kökleri onun dışında).
    #[test]
    fn test_instance_never_writes_real_paths() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("realhome");
        std::fs::create_dir_all(&real).unwrap();
        let env = nuxut_apply::Env { real_home: Some(real.clone()), ..nuxut_apply::Env::for_test(&t.path().join("roots")) };
        let dirs = Dirs { config: real.join(".config"), data: real.join(".local/share"), home: real.clone() };
        let state = state_for(ON, dirs);
        let mut rec = Rec::default();
        let r = sync(&state, &env, &mut rec).report.unwrap();
        assert!(r.written.is_empty() && r.unchanged.is_empty(), "{r:?}");
        assert!(r.errors.is_empty() && r.skipped.len() == setting_count(), "{r:?}");
        assert!(r.skipped.contains(&plan::setting::MODE), "look.mode bütçe hatası değil, kapıda atlanır: {r:?}");
        assert!(rec.0.is_empty());
        assert!(snapshots(&env).is_empty());
        assert_eq!(std::fs::read_dir(&real).unwrap().count(), 0, "gerçek ev dizinine hiçbir şey yazılmadı");
        // Kapalı ortam (ikincil örnek) da hiçbir şey.
        let off = nuxut_apply::Env { disabled: true, ..env };
        let r = sync(&state, &off, &mut rec).report.unwrap();
        assert!(r.written.is_empty() && r.errors.is_empty() && r.skipped.len() == setting_count(), "{r:?}");
    }

    /// Adres bütçesi: look.* ayarlarının her biri, gerçekçi uzun bir yapılandırma yolunda bile tek
    /// işleme sığar; aşırı uzun yolda bölünmez, açık adla (`too_many_fragments`) hiç yazılmaz.
    #[test]
    fn every_look_change_fits_one_transaction() {
        let t = tempfile::tempdir().unwrap();
        // 256 baytlık kök: gerçek ev dizinlerinin çok üstünde.
        let long = t.path().join("a".repeat(120)).join("b".repeat(120));
        let state = state_for(ON, temp_dirs(&long));
        assert!(!state.plan.changes.is_empty());
        for c in &state.plan.changes {
            let req = writer::to_change(c, true).unwrap();
            assert!(writer::fits_one_transaction(&req), "{}: {} parça", c.setting, req.writes.len());
        }
        // Aşırı uzun kök (~3 KiB): look.mode adres bütçesini aşar → yazılmaz, hiçbir dosya/snapshot yok.
        let huge: std::path::PathBuf = (0..12).fold(t.path().join("h"), |p, i| p.join(format!("{i}{}", "x".repeat(250))));
        let env = nuxut_apply::Env::for_test(t.path());
        let mut rec = Rec::default();
        let r = sync(&state_for(ON, temp_dirs(&huge)), &env, &mut rec).report.unwrap();
        assert_eq!(r.errors, [format!("{}: {}", plan::setting::MODE, writer::TOO_MANY_FRAGMENTS)], "{r:?}");
        assert!(snapshots(&env).iter().all(|s| s.setting.as_str() != plan::setting::MODE));
    }

    // -----------------------------------------------------------------------------------------
    // Erişilebilirlik (framework docs/design/accessibility.md §1–3, t124), yalıtılmış uçtan uca.

    /// Azaltılmış hareket iki `settings.ini`'ye `gtk-enable-animations=0` yazar (look.motion; dconf
    /// test örneğinde düşülür, plan testi kapsar); açılınca yalnızca look.motion yeniden yazılır;
    /// kapatma kullanıcının değerine byte-eşit döner.
    #[test]
    fn reduced_motion_applies_and_restores() {
        use plan::setting::MOTION;
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let cfg = dirs.config.clone();
        std::fs::create_dir_all(cfg.join("gtk-3.0")).unwrap();
        let user_ini = "[Settings]\ngtk-enable-animations=true\n";
        std::fs::write(cfg.join("gtk-3.0/settings.ini"), user_ini).unwrap();
        let mut rec = Rec::default();
        let still = "[motion]\nenabled = false\n[desktop]\nunified = true\n";
        let r = sync(&state_for(still, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(r.ok && r.written.contains(&MOTION), "{r:?}");
        for file in ["gtk-3.0/settings.ini", "gtk-4.0/settings.ini"] {
            let ini = read(&cfg.join(file)).unwrap();
            assert!(ini.contains("gtk-enable-animations=0\n") && !ini.contains("=true\ngtk-enable"), "{file}: {ini}");
        }
        // Hareket yeniden açık: yalnız look.motion.
        let r = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert_eq!(r.written, [MOTION], "{r:?}");
        assert!(read(&cfg.join("gtk-4.0/settings.ini")).unwrap().contains("gtk-enable-animations=1\n"));
        assert!(snapshots(&env).iter().any(|s| s.setting.as_str() == MOTION && s.pinned));
        // Kapatma: Nuxut öncesi.
        let back = sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(back.ok && back.written.contains(&MOTION), "{back:?}");
        assert_eq!(read(&cfg.join("gtk-3.0/settings.ini")).unwrap(), user_ini);
        assert!(!cfg.join("gtk-4.0/settings.ini").exists());
    }

    /// Göç: önceki sürüm GTK `gtk-font-name`'e ölçek katılmış pt yazmıştı (size 16 → 12 pt,
    /// look.fonts işlemi). Sonraki tur adı referans boya (10 pt) çevirir ve ölçeği `gtk-xft-dpi`'a
    /// yazar (tek ölçek); kapatma ikisini de kullanıcının ilk hâline döndürür.
    #[test]
    fn legacy_scaled_gtk_font_migrates_to_reference_size() {
        use plan::setting::FONTS;
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let ini_path = dirs.config.join("gtk-3.0/settings.ini");
        std::fs::create_dir_all(ini_path.parent().unwrap()).unwrap();
        let user_ini = "[Settings]\ngtk-font-name=Cantarell 11\n";
        std::fs::write(&ini_path, user_ini).unwrap();
        let big = "[typography]\nsize = 16\n[desktop]\nunified = true\n";
        let state = state_for(big, dirs.clone());
        let fonts = state.plan.changes.iter().find(|c| c.setting == FONTS).unwrap();
        let new_name = fonts
            .writes
            .iter()
            .find_map(|(f, v)| matches!(f, plan::Fragment::IniKey { file, key, .. } if *file == ini_path && key == "gtk-font-name").then(|| v.clone()))
            .unwrap();
        assert!(new_name.ends_with(" 10"), "{new_name}");
        // Eski sürümün yazımı: aynı ad, ölçekli pt.
        let old_name = format!("{} 12", new_name.strip_suffix(" 10").unwrap());
        let old = nuxut_apply::Change::new(nuxut_apply::SettingId::new(FONTS).unwrap(), "eski", nuxut_apply::Source::Shell).write(
            nuxut_apply::Fragment::IniKey { file: ini_path.clone(), section: "Settings".into(), key: "gtk-font-name".into() },
            nuxut_apply::FragValue::Text(old_name.clone()),
        );
        let app = nuxut_apply::Applier::open(env.clone()).with_source(nuxut_apply::Source::Shell);
        assert!(matches!(app.apply(old), nuxut_apply::Outcome::Applied { .. }));
        assert!(read(&ini_path).unwrap().contains(&format!("gtk-font-name={old_name}\n")));

        let mut rec = Rec::default();
        let r = sync(&state, &env, &mut rec).report.unwrap();
        assert!(r.ok && r.written.contains(&FONTS), "{r:?}");
        let ini = read(&ini_path).unwrap();
        assert!(ini.contains(&format!("gtk-font-name={new_name}\n")) && !ini.contains(&old_name), "{ini}");
        assert!(ini.contains("gtk-xft-dpi=120914\n"), "ölçek yalnız xft-dpi'da: {ini}");
        // Aynı girdi: NoOp.
        assert!(sync(&state, &env, &mut rec).report.unwrap().written.is_empty());

        let back = sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(back.ok, "{:?}", back.errors);
        assert_eq!(read(&ini_path).unwrap(), user_ini, "eski ölçekli ad ve xft-dpi gitti, kullanıcınınki döndü");
    }

    /// Yüksek karşıtlık canlı değişimi: yalnız look.mode yeniden yazılır, GTK CSS farklı.
    #[test]
    fn high_contrast_rewrites_only_colors() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let mut rec = Rec::default();
        assert!(sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap().ok);
        let css = read(&dirs.config.join("gtk-4.0/nuxut.css")).unwrap();
        let high = "[colors]\ncontrast = \"high\"\n[desktop]\nunified = true\n";
        let r = sync(&state_for(high, dirs.clone()), &env, &mut rec).report.unwrap();
        assert_eq!(r.written, [plan::setting::MODE], "{r:?}");
        assert_ne!(read(&dirs.config.join("gtk-4.0/nuxut.css")).unwrap(), css);
    }

    // -----------------------------------------------------------------------------------------
    // 2026-10-16 hatası (end4/dots-hyprland kurulu makine): look.mode `externally_managed`, look.icons
    // ve look.fonts saniyede bir yeniden yazılıyordu. docs/design/look-external.md.

    /// Başka aracın yönettiği dosyalar (2026-10-16 kullanıcı ilkesi: devralma varsayılan).
    /// Salt okunur `gtk-3.0/gtk.css` devralınır (satır eklenir, kapatınca içerik + izin geri);
    /// devralınamayan (yazılamayan `gtk-4.0` dizini) bütün look.mode'u düşürmez: o dosyalara
    /// dokunulmaz, kalan parçalar yazılır, rapor dosyayı ve nedeni verir. Art arda ikinci tur hepsi
    /// NoOp (dosya ve snapshot değişmez); kapatma Nuxut öncesine.
    #[test]
    fn external_files_are_taken_over_or_left_out() {
        use plan::setting::MODE;
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let cfg = dirs.config.clone();
        std::fs::create_dir_all(cfg.join("gtk-3.0")).unwrap();
        std::fs::create_dir_all(cfg.join("gtk-4.0")).unwrap();
        let end4_kde = "[General]\nColorScheme=MaterialYouDark\nfont=Google Sans Flex,11,-1,5,500,0,0,0,0,0,0,0,0,0,0,1,Medium\n\n[Icons]\nTheme=breeze-dark\n\n[KDE]\nwidgetStyle=Darkly\n";
        std::fs::write(cfg.join("kdeglobals"), end4_kde).unwrap();
        let css = cfg.join("gtk-3.0/gtk.css");
        let matugen = "/* matugen */\n@define-color accent_color #cdb9fb;\n";
        std::fs::write(&css, matugen).unwrap();
        std::fs::set_permissions(&css, std::fs::Permissions::from_mode(0o444)).unwrap();
        let locked = cfg.join("gtk-4.0");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let mode_of = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

        let mut rec = Rec::default();
        let r = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(r.errors.is_empty() && r.written.contains(&MODE), "look.mode düşmedi: {r:?}");
        assert!(!r.ok, "dış hedef varken ok değil (Ayarlar uyarır)");
        assert!(r.external.len() >= 2, "{:?}", r.external);
        for e in &r.external {
            assert!(e.reason == "dir_not_writable" && e.path.contains("gtk-4.0/"), "{e:?}");
        }
        assert_eq!(read(&css).unwrap(), format!("{matugen}{}\n", gtk::GTK_IMPORT), "salt okunur dosya devralındı");
        assert_eq!(mode_of(&css), 0o644);
        assert_eq!(r.taken_over.len(), 1, "{:?}", r.taken_over);
        assert!(r.taken_over[0].path.ends_with("gtk-3.0/gtk.css") && r.taken_over[0].setting == MODE);
        assert!(read(&cfg.join("gtk-3.0/nuxut.css")).is_some() && read(&cfg.join("kdeglobals")).unwrap().contains("[Colors:Window]"));

        let snaps = snapshots(&env).len();
        let kde = read(&cfg.join("kdeglobals"));
        let again = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(again.written.is_empty() && again.errors.is_empty(), "ikinci tur NoOp: {again:?}");
        assert_eq!(again.external, r.external);
        assert_eq!(again.taken_over, r.taken_over, "devralma sonraki turlarda da görünür");
        assert_eq!((snapshots(&env).len(), read(&cfg.join("kdeglobals"))), (snaps, kde));

        let back = sync(&state_for(OFF, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(back.ok && back.taken_over.is_empty(), "{back:?}");
        assert_eq!(read(&cfg.join("kdeglobals")).unwrap(), end4_kde);
        assert_eq!(read(&css).unwrap(), matugen);
        assert_eq!(mode_of(&css), 0o444, "izin geri");
        assert!(!cfg.join("gtk-3.0/nuxut.css").exists());
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Kullanıcının makinesi (2026-10-16): `gtk-4.0/{gtk.css,gtk-dark.css}` başka bir temanın
    /// dosyasına bağ (Dracula kurulumu), `assets` dizin bağı. Bağlar devralınır ve Nuxut'un
    /// dosyasıyla değiştirilir (tema CSS'i Nuxut renklerini ezerdi; hedefe dokunulmaz), kapatma
    /// bağları aynı metinle geri koyar; `assets` hiç değişmez. Hedef burada salt okunur bir tema
    /// dosyası, test kökünde (test örneğinin kapısı kök dışını okumaz; root'a ait hedef framework
    /// `nuxut-apply` testlerinde).
    #[test]
    fn theme_links_are_replaced_and_restored() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let theme_css = dirs.config.join("tema-kaynagi/Dracula/gtk-4.0/gtk.css");
        std::fs::create_dir_all(theme_css.parent().unwrap()).unwrap();
        std::fs::write(&theme_css, "/* Dracula */\nwindow { background: #282a36; }\n").unwrap();
        std::fs::set_permissions(&theme_css, std::fs::Permissions::from_mode(0o444)).unwrap();
        let root_file = theme_css.as_path();
        let gtk4 = dirs.config.join("gtk-4.0");
        std::fs::create_dir_all(&gtk4).unwrap();
        for name in ["gtk.css", "gtk-dark.css"] {
            std::os::unix::fs::symlink(root_file, gtk4.join(name)).unwrap();
        }
        std::os::unix::fs::symlink("/usr/share", gtk4.join("assets")).unwrap();
        let probe = selection::tests::FakeProbe { links: vec![gtk4.join("gtk.css"), gtk4.join("gtk-dark.css")], ..selection::tests::FakeProbe::full() };
        let state = |text: &str| {
            let (theme, fw) = framework(text, None);
            LookState::compute(&theme, fw, None, dirs.clone(), &probe)
        };
        let mut rec = Rec::default();
        let r = sync(&state(ON), &env, &mut rec).report.unwrap();
        assert!(r.ok && r.external.is_empty(), "{r:?}");
        for name in ["gtk.css", "gtk-dark.css"] {
            let p = gtk4.join(name);
            assert!(!std::fs::symlink_metadata(&p).unwrap().file_type().is_symlink(), "{name} devralındı: {r:?}");
            let text = read(&p).unwrap();
            assert!(text.starts_with("/* Nuxut — ") && text.ends_with(&format!("{}\n", gtk::GTK_IMPORT)), "{text}");
        }
        let mut taken: Vec<String> = r.taken_over.iter().map(|t| t.path.rsplit('/').next().unwrap().to_owned()).collect();
        taken.sort();
        assert_eq!(taken, ["gtk-dark.css", "gtk.css"]);

        let back = sync(&state(OFF), &env, &mut rec).report.unwrap();
        assert!(back.ok && back.taken_over.is_empty(), "{back:?}");
        for name in ["gtk.css", "gtk-dark.css"] {
            assert_eq!(std::fs::read_link(gtk4.join(name)).unwrap(), root_file, "{name} bağı geri");
        }
        assert_eq!(std::fs::read_link(gtk4.join("assets")).unwrap(), std::path::Path::new("/usr/share"));
        assert_eq!(read(&theme_css).unwrap(), "/* Dracula */\nwindow { background: #282a36; }\n", "hedefe hiç yazılmadı");
    }

    /// Nuxut'un kendi yazımı (imleç `~/.icons/default/index.theme` dahil, tema arama kökleri içinde)
    /// kurulu tema izleyicisini tetiklemez: tetiklese her yazım yeni bir tur doğururdu. Denetim:
    /// gerçek bir tema kurulumu sayacı artırır.
    #[test]
    fn own_writes_do_not_wake_theme_watcher() {
        use std::time::Duration;
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        std::fs::create_dir_all(dirs.data.join("icons/Papirus")).unwrap();
        std::fs::write(dirs.data.join("icons/Papirus/index.theme"), "[Icon Theme]\nName=Papirus\n").unwrap();
        let nodata = t.path().join("nodata");
        let roots = watch::roots_from(Some(&dirs.home), Some(&dirs.data), Some(nodata.to_str().unwrap()));
        let (tx, rx) = std::sync::mpsc::channel();
        let opts = watch::WatchOptions { roots, settle: Duration::from_millis(80), max_settle: Duration::from_millis(400) };
        let _w = watch::ThemeWatcher::spawn(opts, move |r| {
            let _ = tx.send(r);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let mut rec = Rec::default();
        let r = sync(&state_for(ON, dirs.clone()), &env, &mut rec).report.unwrap();
        assert!(r.ok && r.written.contains(&plan::setting::CURSOR), "{r:?}");
        assert!(dirs.home.join(".icons/default/index.theme").exists());
        assert!(rx.recv_timeout(Duration::from_millis(1200)).is_err(), "kendi yazımımız tema izleyicisini uyandırdı");

        std::fs::create_dir_all(dirs.data.join("icons/Tela")).unwrap();
        std::fs::write(dirs.data.join("icons/Tela/index.theme"), "[Icon Theme]\nName=Tela\n").unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok(), "denetim: gerçek kurulum sayılır");
    }

    /// Nuxut GTK teması uçtan uca (geçici kök): tema dosyaları yazılır, kullanıcı gtk-3.0/gtk.css'teki
    /// eski `@import nuxut.css` satırı ve nuxut.css kalkar, kapatma hepsini Nuxut öncesine döndürür.
    #[test]
    fn gtk_theme_applies_and_reverts_in_temp_root() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let cfg = dirs.config.clone();
        std::fs::create_dir_all(cfg.join("gtk-3.0")).unwrap();
        // Önceki sürümün bıraktığı satır + dosya (kullanıcı içeriğiyle).
        let user_css = "/* benim */\nwindow { margin: 1px; }\n@import url(\"nuxut.css\");\n";
        std::fs::write(cfg.join("gtk-3.0/gtk.css"), user_css).unwrap();
        std::fs::write(cfg.join("gtk-3.0/nuxut.css"), "/* Nuxut — eski · elle düzenleme bir sonraki uygulamada ezilir */\n").unwrap();
        let base = gtk_theme::GtkBase {
            name: "adw-gtk3-dark".into(),
            dir: "/usr/share/themes/adw-gtk3-dark".into(),
            css3: "@define-color nuxut_window_bg_color #242424;\n".into(),
        };
        let probe = selection::tests::FakeProbe { base: Some(base), ..selection::tests::FakeProbe::full() };
        let state = |text: &str| {
            let (theme, fw) = framework(text, None);
            LookState::compute(&theme, fw, None, dirs.clone(), &probe)
        };
        let mut rec = Rec::default();
        let r = sync(&state(ON), &env, &mut rec).report.unwrap();
        assert!(r.ok && r.errors.is_empty(), "{r:?}");
        let theme = dirs.data.join("themes/Nuxut");
        assert!(read(&theme.join("gtk-3.0/base.css")).unwrap().contains("nuxut_window_bg_color #242424"));
        assert!(read(&theme.join("gtk-3.0/gtk.css")).unwrap().contains("@define-color nuxut_"));
        assert!(read(&dirs.data.join("themes/Nuxut-reload/gtk-3.0/gtk.css")).is_some());
        assert_eq!(read(&cfg.join("gtk-3.0/gtk.css")).unwrap(), "/* benim */\nwindow { margin: 1px; }\n", "eski satır kalktı, kullanıcı içeriği kaldı");
        assert!(!cfg.join("gtk-3.0/nuxut.css").exists());
        assert!(read(&cfg.join("gtk-3.0/settings.ini")).unwrap().contains("gtk-theme-name=Nuxut\n"));
        // Kapatma: Nuxut öncesi (eski satır ve dosya dahil) geri.
        sync(&state(OFF), &env, &mut rec).report.unwrap();
        assert_eq!(read(&cfg.join("gtk-3.0/gtk.css")).unwrap(), user_css);
        assert!(cfg.join("gtk-3.0/nuxut.css").exists());
        assert!(!theme.join("gtk-3.0/gtk.css").exists() && !dirs.data.join("themes/Nuxut-reload/index.theme").exists());
        // Nuxut'un yarattığı ve boş kalan dizinler de gider (nuxut-apply created_dirs); var olan
        // kullanıcı dizinleri kalır.
        assert!(!dirs.data.join("themes").exists(), "boş tema dizinleri kalmadı");
        assert!(cfg.join("gtk-3.0").is_dir());
    }

    /// Ölçüm (elle: `cargo test latency_of_a_colour_change -- --ignored --nocapture`): bir renk
    /// değişiminin geçici kökte uygulanma süresi (dconf ve canlı eylemler test örneğinde yok).
    #[test]
    #[ignore]
    fn latency_of_a_colour_change() {
        let t = tempfile::tempdir().unwrap();
        let env = nuxut_apply::Env::for_test(t.path());
        let dirs = temp_dirs(t.path());
        let mut rec = Rec::default();
        let time = |text: &str, rec: &mut Rec| {
            let state = state_for(text, dirs.clone());
            let start = std::time::Instant::now();
            let r = sync(&state, &env, rec).report.unwrap();
            (start.elapsed(), r.written)
        };
        let (first, w) = time(ON, &mut rec);
        eprintln!("ilk tur: {first:?} {w:?}");
        for (i, surface) in ["#101418", "#18141a", "#141814", "#101418"].iter().enumerate() {
            let text = format!("[colors]\nsurface = \"{surface}\"\nprimary = \"#{:02x}88cc\"\n[desktop]\nunified = true\n", 0x40 + i * 0x20);
            let (d, w) = time(&text, &mut rec);
            eprintln!("renk değişimi {i}: {d:?} {w:?}");
        }
    }
}

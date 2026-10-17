//! Bus bağlantısı, paylaşılan durum ve config izleyici döngüsü.
//!
//! Akış: izleyiciler kurulur → tema yüklenir → nesne sunulur → ad alınır ([`Portal::start`]).
//! [`Portal::run`] iki inotify tanımlayıcısını (`framework.toml` dizini, `palette.toml` dizini)
//! `poll(2)` ile bekler; boşta hiç uyanmaz. Olay gelince kısa bir birleştirme beklemesinden sonra
//! tema yeniden yüklenir ve yalnızca değeri değişen anahtarlar için `SettingChanged` yollanır.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nuxut_config::ConfigWatcher;
use nuxut_theme::{CONFIG_NAME, ThemeLoader, palette_path};
use rustix::event::{PollFd, PollFlags};
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::names::BusName;

use crate::appearance::{Appearance, NAMESPACE, changed_keys};
use crate::chooser::FileChooser;
use crate::settings::{Settings, Shared};
use crate::{BUS_NAME, OBJECT_PATH, SETTINGS_IFACE};

/// Olaydan sonra yeniden yüklemeden önce beklenen süre: aynı kaydın (framework.toml + palette.toml,
/// editörün geçici dosya + yeniden adlandırması) olayları tek yüklemede birleşir.
const COALESCE: Duration = Duration::from_millis(40);

/// Okunan dosyalar.
#[derive(Debug, Clone, Default)]
pub struct Paths {
    /// `framework.toml`.
    pub config: Option<PathBuf>,
    /// Üretilmiş `palette.toml` (`[colors] source = "wallpaper"` iken).
    pub palette: Option<PathBuf>,
}

impl Paths {
    /// Runtime ile aynı yollar: `$XDG_CONFIG_HOME/nuxut/framework.toml`,
    /// `$XDG_STATE_HOME/nuxut/palette.toml`.
    pub fn from_env() -> Self {
        Self { config: nuxut_config::config_path(CONFIG_NAME), palette: palette_path() }
    }
}

/// Çalışan arka uç.
#[derive(Debug)]
pub struct Portal {
    conn: Connection,
    state: Shared,
    loader: ThemeLoader,
    watchers: Vec<ConfigWatcher>,
}

/// Temayı yükler. framework.toml bozuksa: ilk yüklemede saydam (runtime'ın `broken_config`'i gibi
/// birleşik görünüm kapalı sayılır), çalışırken son geçerli değer korunur.
fn load(loader: &mut ThemeLoader, last: Option<Option<Appearance>>) -> Option<Appearance> {
    match loader.load() {
        Ok(l) => Appearance::from_theme(&l.value),
        Err(e) => {
            tracing::warn!("framework.toml okunamadı: {e}");
            last.unwrap_or_else(|| Appearance::from_theme(&nuxut_theme::Theme::broken_config()))
        }
    }
}

/// Dosyanın dizinini, yalnızca o dosya adı için izler. Kurulamazsa uyarı (Read yine çalışır,
/// yalnızca canlı sinyal yok).
fn watch(file: Option<&Path>) -> Option<ConfigWatcher> {
    let file = file?;
    let (dir, name) = (file.parent()?, file.file_name()?.to_str()?);
    ConfigWatcher::new(dir, &[name]).map_err(|e| tracing::warn!("{} izlenemiyor: {e}", dir.display())).ok()
}

impl Portal {
    /// İzleyicileri kurar, temayı yükler, nesneyi sunar ve [`BUS_NAME`]'i alır. `builder`: oturum
    /// bus'ı (`Builder::session()`) ya da testte özel bus.
    pub fn start(builder: Builder<'_>, paths: Paths) -> zbus::Result<Self> {
        Self::start_with(builder, paths, FileChooser::new(crate::chooser::iface::locate_bin()))
    }

    /// [`Self::start`], dosya seçici nesnesi verilerek (testlerde sahte seçici ikilisi).
    pub fn start_with(builder: Builder<'_>, paths: Paths, chooser: FileChooser) -> zbus::Result<Self> {
        // Önce izleyiciler: yükleme ile izleme arasındaki değişiklik kaçmasın.
        let watchers: Vec<ConfigWatcher> = [paths.config.as_deref(), paths.palette.as_deref()].into_iter().filter_map(watch).collect();
        let mut loader = ThemeLoader::with_paths(paths.config, paths.palette);
        let state: Shared = Shared::default();
        *state.lock().unwrap_or_else(|e| e.into_inner()) = load(&mut loader, None);
        let conn = builder.serve_at(OBJECT_PATH, Settings::new(state.clone()))?.serve_at(OBJECT_PATH, chooser)?.build()?;
        // Ad devredilmez ve devralınmaz, kuyruğa da girilmez: ikinci örnek hemen çıkar, çalışan
        // örnek (xdg-desktop-portal'ın bağlı olduğu) yerinden edilmez.
        match conn.request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())? {
            RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => {}
            _ => return Err(zbus::Error::NameTaken),
        }
        Ok(Self { conn, state, loader, watchers })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Şu anki değerler (`None`: saydam).
    pub fn current(&self) -> Option<Appearance> {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Yeniden yükler; durum güncellendikten sonra değişen her anahtar için bir `SettingChanged`
    /// yollar. Yollanan anahtarları döndürür.
    pub fn reload(&mut self) -> Vec<&'static str> {
        let old = self.current();
        let new = load(&mut self.loader, Some(old));
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = new;
        let keys = changed_keys(old.as_ref(), new.as_ref());
        let Some(new) = new else { return keys };
        for key in &keys {
            let Some(value) = new.value(key) else { continue };
            let body = (NAMESPACE, *key, &*value);
            if let Err(e) = self.conn.emit_signal(None::<BusName<'_>>, OBJECT_PATH, SETTINGS_IFACE, "SettingChanged", &body) {
                tracing::warn!("SettingChanged yollanamadı ({key}): {e}");
            }
        }
        if !keys.is_empty() {
            tracing::info!(?keys, "görünüm değişti");
        }
        keys
    }

    /// Bloklar: config olaylarını bekler ve [`Self::reload`] eder. Yalnızca poll hatasında döner.
    /// İzleyici yoksa sonsuza dek uyur (D-Bus yanıtları zbus'ın kendi thread'inde).
    pub fn run(mut self) -> std::io::Result<()> {
        if self.watchers.is_empty() {
            loop {
                std::thread::park();
            }
        }
        loop {
            {
                let mut fds: Vec<PollFd<'_>> = self.watchers.iter().map(|w| PollFd::from_borrowed_fd(w.fd(), PollFlags::IN)).collect();
                match rustix::event::poll(&mut fds, None) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            std::thread::sleep(COALESCE);
            // Her izleyici boşaltılır (kısa devre yok).
            let changed = self.watchers.iter_mut().fold(false, |any, w| !w.changed_files().is_empty() | any);
            if changed {
                self.reload();
            }
        }
    }
}

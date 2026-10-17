//! `org.freedesktop.impl.portal.Settings` özel bir dbus-daemon üzerinde: ReadAll süzgeci, Read,
//! bilinmeyen anahtar hatası, sürüm, saydam kip ve dosya değişince tek `SettingChanged`.

mod common;

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use nuxut_portal::appearance::{ACCENT_COLOR, COLOR_SCHEME, KEYS, NAMESPACE, REDUCED_MOTION};
use nuxut_portal::service::{Paths, Portal};
use nuxut_portal::{BUS_NAME, OBJECT_PATH, SETTINGS_IFACE};
use std::collections::HashMap;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::zvariant::OwnedValue;

use common::PrivateBus;

type All = HashMap<String, HashMap<String, OwnedValue>>;

struct Env {
    _dir: tempfile::TempDir,
    config: PathBuf,
}

fn env(framework: &str) -> (Env, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config/nuxut/framework.toml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, framework).unwrap();
    let palette = dir.path().join("state/nuxut/palette.toml");
    let paths = Paths { config: Some(config.clone()), palette: Some(palette) };
    (Env { _dir: dir, config }, paths)
}

/// Ayarlar'ın yaptığı gibi atomik yazım (geçici dosya + yeniden adlandırma).
fn write_atomic(path: &Path, text: &str) {
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

fn proxy(conn: &Connection) -> Proxy<'_> {
    Proxy::new(conn, BUS_NAME, OBJECT_PATH, SETTINGS_IFACE).unwrap()
}

fn read_all(p: &Proxy<'_>, namespaces: &[&str]) -> All {
    p.call("ReadAll", &(namespaces.to_vec(),)).unwrap()
}

fn read(p: &Proxy<'_>, ns: &str, key: &str) -> zbus::Result<OwnedValue> {
    p.call("Read", &(ns, key))
}

fn not_found(r: zbus::Result<OwnedValue>) -> bool {
    match r {
        Err(zbus::Error::MethodError(name, _, _)) => name.as_str() == "org.freedesktop.portal.Error.NotFound",
        _ => false,
    }
}

#[test]
fn read_all_glob_read_and_errors() {
    let Some(bus) = PrivateBus::start() else { return };
    let (_env, paths) = env("[colors]\nmode = \"light\"\nprimary = \"#3366cc\"\n");
    let _portal = Portal::start(bus.builder(), paths).unwrap();
    let client = bus.connect();
    let p = proxy(&client);

    for patterns in [&[][..], &[""], &["org.freedesktop.*"], &["org.gnome.*", NAMESPACE]] {
        let all = read_all(&p, patterns);
        assert_eq!(all.len(), 1, "{patterns:?}");
        let ns = &all[NAMESPACE];
        assert_eq!(ns.len(), KEYS.len(), "{patterns:?}");
        assert_eq!(u32::try_from(ns[COLOR_SCHEME].try_clone().unwrap()).unwrap(), 2);
    }
    assert!(read_all(&p, &["org.gnome.*"]).is_empty());
    assert!(read_all(&p, &["org.freedesktop"]).is_empty());

    assert_eq!(u32::try_from(read(&p, NAMESPACE, COLOR_SCHEME).unwrap()).unwrap(), 2);
    assert_eq!(u32::try_from(read(&p, NAMESPACE, REDUCED_MOTION).unwrap()).unwrap(), 0);
    let accent = read(&p, NAMESPACE, ACCENT_COLOR).unwrap();
    assert_eq!(accent.value_signature().to_string(), "(ddd)");
    let (r, g, b) = <(f64, f64, f64)>::try_from(accent).unwrap();
    assert_eq!([r, g, b], [0x33 as f64 / 255.0, 0x66 as f64 / 255.0, 0xcc as f64 / 255.0]);

    assert!(not_found(read(&p, NAMESPACE, "no-such-key")));
    assert!(not_found(read(&p, "org.gnome.desktop.interface", "color-scheme")));

    let version: u32 = p.get_property("version").unwrap();
    assert_eq!(version, 2);
}

/// Birleşik görünüm kapalı: hiçbir anahtar bilinmez (gtk arka ucuna düşülür).
#[test]
fn unified_off_is_transparent() {
    let Some(bus) = PrivateBus::start() else { return };
    let (_env, paths) = env("[desktop]\nunified = false\n");
    let _portal = Portal::start(bus.builder(), paths).unwrap();
    let client = bus.connect();
    let p = proxy(&client);
    assert!(read_all(&p, &[]).is_empty());
    assert!(not_found(read(&p, NAMESPACE, COLOR_SCHEME)));
}

/// Sinyalleri bir kanala aktarır (zaman aşımlı bekleme için).
fn signals(conn: &Connection) -> mpsc::Receiver<(String, String, OwnedValue)> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(SETTINGS_IFACE)
        .unwrap()
        .member("SettingChanged")
        .unwrap()
        .build();
    let iter = MessageIterator::for_match_rule(rule, conn, Some(64)).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for msg in iter {
            let Ok(msg) = msg else { break };
            let Ok(body) = msg.body().deserialize::<(String, String, OwnedValue)>() else { continue };
            if tx.send(body).is_err() {
                break;
            }
        }
    });
    rx
}

const WAIT: Duration = Duration::from_secs(5);
/// Fazladan sinyal yokluğu için bekleme (birleştirme 40 ms; bol pay).
const QUIET: Duration = Duration::from_millis(700);

#[test]
fn file_change_emits_one_signal_per_changed_key() {
    let Some(bus) = PrivateBus::start() else { return };
    let (env, paths) = env("[colors]\nmode = \"dark\"\n");
    let portal = Portal::start(bus.builder(), paths).unwrap();
    std::thread::spawn(move || portal.run());
    let client = bus.connect();
    let rx = signals(&client);
    let p = proxy(&client);

    // Yalnızca hareket değişir → tek sinyal (reduced-motion = 1).
    write_atomic(&env.config, "[colors]\nmode = \"dark\"\n[motion]\npreset = \"instant\"\n");
    let (ns, key, value) = rx.recv_timeout(WAIT).expect("SettingChanged gelmedi");
    assert_eq!((ns.as_str(), key.as_str()), (NAMESPACE, REDUCED_MOTION));
    assert_eq!(u32::try_from(value).unwrap(), 1);
    assert!(rx.recv_timeout(QUIET).is_err(), "fazladan sinyal");
    // Sinyal durum güncellendikten sonra: okuma yeni değeri verir.
    assert_eq!(u32::try_from(read(&p, NAMESPACE, REDUCED_MOTION).unwrap()).unwrap(), 1);

    // Aynı içerik yeniden yazılır (yerinde, CLOSE_WRITE): değer değişmedi → sinyal yok.
    std::fs::write(&env.config, "[colors]\nmode = \"dark\"\n[motion]\npreset = \"instant\"\n").unwrap();
    assert!(rx.recv_timeout(QUIET).is_err(), "değişmeyen değer için sinyal");

    // Yalnızca vurgu değişir.
    write_atomic(&env.config, "[colors]\nmode = \"dark\"\nprimary = \"#3366cc\"\n[motion]\npreset = \"instant\"\n");
    let (_, key, _) = rx.recv_timeout(WAIT).expect("accent-color sinyali gelmedi");
    assert_eq!(key, ACCENT_COLOR);
    assert!(rx.recv_timeout(QUIET).is_err(), "fazladan sinyal");

    // Kip değişir: color-scheme = 2 gelir (açık kipin varsayılan vurgusu da değişebilir).
    write_atomic(&env.config, "[colors]\nmode = \"light\"\nprimary = \"#3366cc\"\n[motion]\npreset = \"instant\"\n");
    let (_, key, value) = rx.recv_timeout(WAIT).expect("color-scheme sinyali gelmedi");
    assert_eq!(key, COLOR_SCHEME);
    assert_eq!(u32::try_from(value).unwrap(), 2);
    assert!(rx.recv_timeout(QUIET).is_err(), "fazladan sinyal");

    // Bozuk dosya: son geçerli değerler korunur, sinyal yok.
    write_atomic(&env.config, "[colors\nmode = ");
    assert!(rx.recv_timeout(QUIET).is_err(), "bozuk dosyada sinyal");
    assert_eq!(u32::try_from(read(&p, NAMESPACE, COLOR_SCHEME).unwrap()).unwrap(), 2);

    // Birleşik görünüm kapanır: saydam, sinyal yok (portal anahtar kaldırmayı bildiremez).
    write_atomic(&env.config, "[desktop]\nunified = false\n");
    assert!(rx.recv_timeout(QUIET).is_err(), "saydama geçişte sinyal");
    assert!(not_found(read(&p, NAMESPACE, COLOR_SCHEME)));

    // Yeniden açılır: bütün anahtarlar birer kez.
    write_atomic(&env.config, "[colors]\nmode = \"light\"\n");
    let mut keys: Vec<String> = (0..KEYS.len()).map(|_| rx.recv_timeout(WAIT).expect("sinyal eksik").1).collect();
    keys.sort();
    let mut want: Vec<String> = KEYS.iter().map(|k| (*k).to_owned()).collect();
    want.sort();
    assert_eq!(keys, want);
    assert!(rx.recv_timeout(QUIET).is_err(), "fazladan sinyal");
}

/// Bus adı başka süreçteyse ikinci örnek başlamaz (ad paylaşılmaz).
#[test]
fn second_instance_fails_to_own_name() {
    let Some(bus) = PrivateBus::start() else { return };
    let (_env, paths) = env("");
    let _first = Portal::start(bus.builder(), paths.clone()).unwrap();
    assert!(Portal::start(bus.builder(), paths).is_err());
}

/// Duvar kağıdı kaynağında `palette.toml` (durum dizini, ikinci izleyici) değişince kip izlenir.
#[test]
fn palette_file_change_is_watched() {
    let Some(bus) = PrivateBus::start() else { return };
    let (env, paths) = env("[colors]\nsource = \"wallpaper\"\n");
    let palette = paths.palette.clone().unwrap();
    let portal = Portal::start(bus.builder(), paths).unwrap();
    std::thread::spawn(move || portal.run());
    let client = bus.connect();
    let rx = signals(&client);
    let p = proxy(&client);
    assert_eq!(u32::try_from(read(&p, NAMESPACE, COLOR_SCHEME).unwrap()).unwrap(), 1);

    write_atomic(&palette, "[meta]\nmode = \"light\"\n");
    let mut keys = Vec::new();
    while let Ok((_, key, value)) = rx.recv_timeout(QUIET) {
        if key == COLOR_SCHEME {
            assert_eq!(u32::try_from(value).unwrap(), 2);
        }
        keys.push(key);
    }
    assert!(keys.iter().any(|k| k == COLOR_SCHEME), "{keys:?}");
    assert_eq!(u32::try_from(read(&p, NAMESPACE, COLOR_SCHEME).unwrap()).unwrap(), 2);
    drop(env);
}

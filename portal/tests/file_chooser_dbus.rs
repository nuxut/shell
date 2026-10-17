//! `org.freedesktop.impl.portal.FileChooser` özel bir dbus-daemon üzerinde, sahte seçici betiğiyle:
//! seçim → `file://` URI, vazgeçme, `Request.Close` ile iptal, eşzamanlı iki istek, geçersiz
//! sonuç ve seçici yokken 2.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use nuxut_portal::chooser::FileChooser;
use nuxut_portal::chooser::protocol::encode_path;
use nuxut_portal::service::{Paths, Portal};
use nuxut_portal::{BUS_NAME, FILE_CHOOSER_IFACE, OBJECT_PATH};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use common::PrivateBus;

type Results = HashMap<String, OwnedValue>;

/// Betiği alt süreçte yazar (ETXTBSY; 3003a95).
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let src = dir.join(format!("{name}.src"));
    std::fs::write(&src, format!("#!/bin/sh\n{body}\n")).unwrap();
    let bin = dir.join(name);
    assert!(Command::new("cp").arg(&src).arg(&bin).status().unwrap().success());
    assert!(Command::new("chmod").arg("+x").arg(&bin).status().unwrap().success());
    bin
}

struct Setup {
    bus: PrivateBus,
    _portal: Portal,
    dir: tempfile::TempDir,
}

fn setup(body: &str) -> Option<Setup> {
    let bus = PrivateBus::start()?;
    let dir = tempfile::tempdir().unwrap();
    let body = body.replace("@DIR@", &encode_path(dir.path()));
    let bin = script(dir.path(), "chooser", &body);
    let paths = Paths { config: None, palette: None };
    let portal = Portal::start_with(bus.builder(), paths, FileChooser::new(Some(bin))).unwrap();
    Some(Setup { bus, _portal: portal, dir })
}

fn call(conn: &Connection, method: &str, handle: &str, options: HashMap<&str, Value<'_>>) -> (u32, Results) {
    let p = Proxy::new(conn, BUS_NAME, OBJECT_PATH, FILE_CHOOSER_IFACE).unwrap();
    p.call(method, &(ObjectPath::try_from(handle).unwrap(), "org.example.App", "", "Başlık", options)).unwrap()
}

fn uris(r: &Results) -> Vec<String> {
    Vec::<String>::try_from(r["uris"].try_clone().unwrap()).unwrap()
}

#[test]
fn open_file_returns_file_uri() {
    let Some(s) = setup(r#"read -r req; printf '{"paths":["@DIR@/r%%C3%%A9sum%%C3%%A9%%201.txt"],"choices":[],"filter":null}\n'"#) else { return };
    std::fs::write(s.dir.path().join("résumé 1.txt"), "x").unwrap();
    let conn = s.bus.connect();
    let (code, res) = call(&conn, "OpenFile", "/org/freedesktop/portal/desktop/request/1_1/t1", HashMap::new());
    assert_eq!(code, 0);
    let expect = format!("file://{}/r%C3%A9sum%C3%A9%201.txt", encode_path(s.dir.path()));
    assert_eq!(uris(&res), vec![expect]);
}

#[test]
fn cancel_and_failures() {
    let Some(s) = setup("read -r req; exit 1") else { return };
    let conn = s.bus.connect();
    assert_eq!(call(&conn, "OpenFile", "/r/a", HashMap::new()).0, 1);

    // Var olmayan yol → doğrulama reddeder → 2.
    let Some(s) = setup(r#"read -r req; echo '{"paths":["/yok/yok"],"choices":[],"filter":null}'"#) else { return };
    let conn = s.bus.connect();
    assert_eq!(call(&conn, "OpenFile", "/r/b", HashMap::new()).0, 2);

    let bus = PrivateBus::start().unwrap();
    let _portal = Portal::start_with(bus.builder(), Paths::default(), FileChooser::new(Some("/yok/nuxut-filechooser".into()))).unwrap();
    assert_eq!(call(&bus.connect(), "SaveFile", "/r/c", HashMap::new()).0, 2);
}

#[test]
fn save_files_in_folder() {
    let Some(s) = setup(
        r#"read -r req; case "$req" in *'"mode":"save_files"'*'"files":["a.txt","b.txt"]'*) printf '{"paths":["@DIR@/a.txt","@DIR@/b%%20%%282%%29.txt"],"choices":[["fmt","pdf"]],"filter":null}\n' ;; *) exit 2 ;; esac"#,
    ) else {
        return;
    };
    let conn = s.bus.connect();
    let mut o: HashMap<&str, Value<'_>> = HashMap::new();
    o.insert("files", Value::from(vec![b"a.txt\0".to_vec(), b"b.txt\0".to_vec()]));
    let choices = vec![("fmt".to_owned(), "Biçim".to_owned(), vec![("pdf".to_owned(), "PDF".to_owned()), ("odt".to_owned(), "ODT".to_owned())], "odt".to_owned())];
    o.insert("choices", Value::from(choices));
    let (code, res) = call(&conn, "SaveFiles", "/r/d", o);
    assert_eq!(code, 0);
    assert_eq!(uris(&res).len(), 2);
    let ch = Vec::<(String, String)>::try_from(res["choices"].try_clone().unwrap()).unwrap();
    assert_eq!(ch, vec![("fmt".to_owned(), "pdf".to_owned())]);
}

#[test]
fn close_cancels_pending_request_and_closes_chooser() {
    // stdin kapanınca çıkan seçici: işaret dosyası iptalin gerçekten seçiciye ulaştığını gösterir.
    let Some(s) = setup("read -r req; cat >/dev/null; : > \"$(dirname \"$0\")/closed\"; exit 1") else { return };
    let address = s.bus.address.clone();
    let handle = "/org/freedesktop/portal/desktop/request/1_1/kapat";
    let pending = std::thread::spawn(move || {
        let conn = zbus::blocking::connection::Builder::address(address.as_str()).unwrap().build().unwrap();
        call(&conn, "OpenFile", handle, HashMap::new())
    });
    let closer = s.bus.connect();
    let req = Proxy::new(&closer, BUS_NAME, handle, "org.freedesktop.impl.portal.Request").unwrap();
    // Request nesnesi çağrı sürerken görünür.
    let start = Instant::now();
    loop {
        match req.call::<_, _, ()>("Close", &()) {
            Ok(()) => break,
            Err(_) if start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("Close: {e}"),
        }
    }
    let (code, res) = pending.join().unwrap();
    assert_eq!(code, 1);
    assert!(res.is_empty());
    assert!(s.dir.path().join("closed").exists());
    // Çağrı bitince nesne kalkar.
    assert!(req.call::<_, _, ()>("Close", &()).is_err());
}

#[test]
fn concurrent_requests_are_independent() {
    // İlki 1 sn bekler, ikincisi hemen yanıtlar: ikinci çağrı birincinin bitmesini beklemez.
    let Some(s) = setup(
        r#"read -r req; case "$req" in *'"title":"yavaş"'*) sleep 1 ;; esac; printf '{"paths":["@DIR@"],"choices":[],"filter":null}\n'"#,
    ) else {
        return;
    };
    let address = s.bus.address.clone();
    let slow = std::thread::spawn(move || {
        let conn = zbus::blocking::connection::Builder::address(address.as_str()).unwrap().build().unwrap();
        let p = Proxy::new(&conn, BUS_NAME, OBJECT_PATH, FILE_CHOOSER_IFACE).unwrap();
        let mut o: HashMap<&str, Value<'_>> = HashMap::new();
        o.insert("directory", Value::from(true));
        let r: (u32, Results) = p.call("OpenFile", &(ObjectPath::try_from("/r/slow").unwrap(), "", "", "yavaş", o)).unwrap();
        (r.0, Instant::now())
    });
    std::thread::sleep(Duration::from_millis(100));
    let conn = s.bus.connect();
    let mut o: HashMap<&str, Value<'_>> = HashMap::new();
    o.insert("directory", Value::from(true));
    let (code, _) = call(&conn, "OpenFile", "/r/fast", o);
    let fast_done = Instant::now();
    assert_eq!(code, 0);
    let (slow_code, slow_done) = slow.join().unwrap();
    assert_eq!(slow_code, 0);
    assert!(fast_done < slow_done);
}

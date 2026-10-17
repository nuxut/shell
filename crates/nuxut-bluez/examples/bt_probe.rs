//! Uçtan uca deneme istemcisi (Ayarlar'ın kullanım kalıbıyla): ajansız,
//! `NUXUT_BLUEZ_BUS=session` zorunlu (gerçek sistem BlueZ'ine asla gitmez). `fake_bluez` ile
//! birlikte `dbus-run-session` içinde çalıştırılır.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use nuxut_bluez::{BluetoothEvent, BluetoothOptions, BluetoothService, BluetoothState, BusKind};

fn wait(rx: &Receiver<BluetoothEvent>, what: &str, pred: impl Fn(&BluetoothEvent) -> bool) -> BluetoothEvent {
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(e) if pred(&e) => return e,
            Ok(_) => {}
            Err(_) => panic!("bt_probe: {what}: zaman aşımı"),
        }
    }
}

fn state(rx: &Receiver<BluetoothEvent>, what: &str, pred: impl Fn(&BluetoothState) -> bool) -> BluetoothState {
    match wait(rx, what, |e| matches!(e, BluetoothEvent::State(s) if pred(s))) {
        BluetoothEvent::State(s) => s,
        _ => unreachable!(),
    }
}

fn main() {
    let bus = nuxut_bluez::bus_from_env();
    if bus != BusKind::Session {
        eprintln!("bt_probe: yalnız NUXUT_BLUEZ_BUS=session ile (sahte BlueZ, dbus-run-session)");
        std::process::exit(2);
    }
    let (tx, rx) = channel();
    let options = BluetoothOptions { bus, ..BluetoothOptions::default() };
    let svc = BluetoothService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .expect("servis");
    let s = state(&rx, "ilk okuma", |s| s.available && s.devices.len() >= 2);
    let adapter = s.adapter().expect("adaptör").path.clone();
    for d in &s.devices {
        println!("bt_probe: cihaz {} eşleşmiş={} güvenilir={} engelli={}", d.name, d.paired, d.trusted, d.blocked);
    }
    svc.set_adapter_alias(&adapter, "Nuxut deneme");
    svc.set_discoverable(&adapter, true, 60);
    let s = state(&rx, "ad + görünürlük", |s| {
        s.adapter().is_some_and(|a| a.name == "Nuxut deneme" && a.discoverable && a.discoverable_timeout == 60)
    });
    println!("bt_probe: adaptör {:?}", s.adapter().map(|a| (&a.name, a.discoverable, a.discoverable_timeout)));
    if let Some(bad) = s.devices.iter().find(|d| d.name == "Hoparlör") {
        svc.pair(&bad.path);
        if let BluetoothEvent::Failed { error, .. } =
            wait(&rx, "eşleştirme hatası", |e| matches!(e, BluetoothEvent::Failed { action: "pair", .. }))
        {
            println!("bt_probe: Hoparlör eşleştirme hatası türü: {error:?}");
        }
    }
    if let Some(k) = s.devices.iter().find(|d| d.name == "Klavye") {
        svc.set_blocked(&k.path, true);
        state(&rx, "engel", |s| s.devices.iter().any(|d| d.name == "Klavye" && d.blocked));
        println!("bt_probe: Klavye engellendi");
    }
    println!("bt_probe: tamam");
}

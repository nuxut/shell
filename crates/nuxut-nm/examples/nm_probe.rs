//! Uçtan uca deneme istemcisi (Ayarlar'ın kullanım kalıbıyla): ajansız, `NUXUT_NM_BUS=session`
//! zorunlu (gerçek sistem NM'ine asla gitmez). `fake_nm` ile birlikte `dbus-run-session` içinde
//! çalıştırılır: durum özeti, cihaz ayrıntıları, gizli ağ, WireGuard içe aktarma, ölçülü yaması.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use nuxut_nm::{BusKind, Metered, NetworkEvent, NetworkOptions, NetworkService, NetworkState, Secret, Security};

// Sentetik örnek anahtar çifti (okunur ASCII metnin base64 hâli): gerçek gizli bilgi değil.
const WG: &str = "[Interface]\nPrivateKey = bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHJpdmF0ZS1rMDE=\nAddress = 10.8.0.2/24\n\
                  DNS = 10.8.0.1\n[Peer]\nPublicKey = bnV4dXQtd2lyZWd1YXJkLXRlc3QtcHVibGljLWtleTE=\n\
                  AllowedIPs = 0.0.0.0/0\nEndpoint = vpn.example.com:51820\n";

fn wait(rx: &Receiver<NetworkEvent>, what: &str, pred: impl Fn(&NetworkEvent) -> bool) -> NetworkEvent {
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(e) if pred(&e) => return e,
            Ok(NetworkEvent::Failed { action, error, .. }) => panic!("nm_probe: {what}: {action} başarısız: {error}"),
            Ok(_) => {}
            Err(_) => panic!("nm_probe: {what}: zaman aşımı"),
        }
    }
}

fn state(rx: &Receiver<NetworkEvent>, what: &str, pred: impl Fn(&NetworkState) -> bool) -> NetworkState {
    match wait(rx, what, |e| matches!(e, NetworkEvent::State(s) if pred(s))) {
        NetworkEvent::State(s) => s,
        _ => unreachable!(),
    }
}

fn main() {
    let bus = nuxut_nm::bus_from_env();
    if bus != BusKind::Session {
        eprintln!("nm_probe: yalnız NUXUT_NM_BUS=session ile (sahte NM, dbus-run-session)");
        std::process::exit(2);
    }
    let (tx, rx) = channel();
    let options = NetworkOptions { bus, ..NetworkOptions::default() };
    let svc = NetworkService::spawn(options, move |e| {
        let _ = tx.send(e);
    })
    .expect("servis");
    let s = state(&rx, "ilk okuma", |s| s.available && !s.saved.is_empty() && !s.devices.is_empty());
    for ap in &s.access_points {
        println!("nm_probe: AP {} {:?} {}%", ap.ssid, ap.security, ap.strength);
    }
    for c in &s.saved {
        println!("nm_probe: kayıtlı {} {:?} autoconnect={} {:?}", c.id, c.kind, c.autoconnect, c.metered);
    }
    for d in &s.devices {
        svc.device_details(&d.path);
        if let NetworkEvent::DeviceDetails(x) = wait(&rx, "ayrıntı", |e| matches!(e, NetworkEvent::DeviceDetails(_))) {
            let ips: Vec<String> = x.ipv4.iter().chain(&x.ipv6).map(ToString::to_string).collect();
            println!("nm_probe: cihaz {} {:?} mac={:?} hız={:?} ip={ips:?} dns={:?}", x.interface, x.kind, x.hw_address, x.speed_mbps, x.dns);
        }
    }
    svc.connect_hidden(b"Gizli", Security::WpaPsk, Some(Secret::new("gizli-sifre")));
    wait(&rx, "gizli ağ", |e| matches!(e, NetworkEvent::Done { action: "connect_hidden", .. }));
    println!("nm_probe: gizli ağ eklendi");
    svc.import_wireguard("probe-wg", &Secret::new(WG)).expect("geçerli yapılandırma");
    wait(&rx, "wireguard", |e| matches!(e, NetworkEvent::Done { action: "import_wireguard", .. }));
    let s = state(&rx, "VPN", |s| s.vpns().any(|v| v.id == "probe-wg"));
    println!("nm_probe: VPN'ler {:?}", s.vpns().map(|v| v.id.as_str()).collect::<Vec<_>>());
    let wifi = s.saved.iter().find(|c| c.ssid.is_some()).expect("kayıtlı Wi-Fi").path.clone();
    svc.set_metered(&wifi, Metered::Yes);
    wait(&rx, "ölçülü", |e| matches!(e, NetworkEvent::Done { action: "metered", .. }));
    state(&rx, "ölçülü durum", |s| s.saved.iter().any(|c| c.path == wifi && c.metered == Metered::Yes));
    println!("nm_probe: tamam");
}

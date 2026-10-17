//! Bildirim uçtan uca testleri için ortak yardımcılar: özel `dbus-daemon` (servis dizini
//! yok), zbus proxy'si, sinyal dinleyicisi, sunucu başlatma ve olay bekleme.
//! `notifications_dbus` ve `notifications_memory` ikilileri bunu `#[path]` ile paylaşır.

// Her test ikilisi yalnızca ihtiyacı olanı kullanır.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use nuxut_shell::services::notifications::server::INTERFACE;
use nuxut_shell::services::notifications::{NotificationServer, NotifyEvent as Ev, NotifyIcon, ServerOptions};
use zbus::blocking;
use zbus::message::Type as MsgType;
use zbus::zvariant::Value;

/// Canlılık sınırı: beklenen olay/sonuç için üst sınır, süre varsayımı DEĞİL — beklemeler sonuç
/// gelince hemen döner; bu yalnızca takılan testi sonunda düşürür. Derleme yükü altında (aynı
/// makinede eşzamanlı derlemeler) gerçek işçi süreci başlatıp çözmek tek başına birkaç saniyeyi aşabilir.
pub const LIVENESS: Duration = Duration::from_secs(120);
const WAIT: Duration = LIVENESS;

/// Yalnızca bu test için `dbus-daemon`. Servis dizini yok: hiçbir şey D-Bus ile etkinleşmez.
pub struct PrivateBus {
    child: Child,
    pub address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    pub fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>"#,
                dir.path().display()
            ),
        )
        .unwrap();
        let mut child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("dbus-daemon");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        Self { child, address: line.trim().to_owned(), _dir: dir }
    }

    pub fn options(&self) -> ServerOptions {
        ServerOptions { address: Some(self.address.clone()), ..ServerOptions::default() }
    }

    pub fn client(&self) -> blocking::Connection {
        blocking::connection::Builder::address(self.address.as_str()).unwrap().build().unwrap()
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
pub trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
    fn close_notification(&self, id: u32) -> zbus::Result<()>;
    fn get_capabilities(&self) -> zbus::Result<Vec<String>>;
    fn get_server_information(&self) -> zbus::Result<(String, String, String, String)>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Sig {
    Closed(u32, u32),
    Action(u32, String),
    Token(u32, String),
}

/// İstemci tarafında sunucunun sinyallerini toplar. Thread bus kapanınca biter.
pub fn signals(conn: &blocking::Connection) -> Receiver<Sig> {
    let rule = zbus::MatchRule::builder().msg_type(MsgType::Signal).interface(INTERFACE).unwrap().build();
    let iter = blocking::MessageIterator::for_match_rule(rule, conn, None).unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for msg in iter.flatten() {
            let header = msg.header();
            let body = msg.body();
            let sig = match header.member().map(|m| m.as_str()) {
                Some("NotificationClosed") => body.deserialize::<(u32, u32)>().map(|(i, r)| Sig::Closed(i, r)),
                Some("ActionInvoked") => body.deserialize::<(u32, String)>().map(|(i, k)| Sig::Action(i, k)),
                Some("ActivationToken") => body.deserialize::<(u32, String)>().map(|(i, t)| Sig::Token(i, t)),
                _ => continue,
            };
            if let Ok(sig) = sig
                && tx.send(sig).is_err()
            {
                return;
            }
        }
    });
    rx
}

pub fn spawn(options: ServerOptions) -> (NotificationServer, Receiver<Ev>) {
    let (tx, rx) = mpsc::channel();
    let server = NotificationServer::spawn_with(options, move |e| {
        let _ = tx.send(e);
    })
    .expect("sunucu başlamalı");
    (server, rx)
}

/// `pred` tutan ilk olayı bekler; aradakiler atılır.
pub fn wait_for<T: std::fmt::Debug>(rx: &Receiver<T>, what: &str, mut pred: impl FnMut(&T) -> bool) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(v) if pred(&v) => return v,
            Ok(_) => {}
            Err(_) => panic!("beklenen gelmedi: {what}"),
        }
    }
}

pub fn added(rx: &Receiver<Ev>) -> nuxut_shell::services::notifications::Notification {
    match wait_for(rx, "Added", |e| matches!(e, Ev::Added(_))) {
        Ev::Added(n) => n,
        _ => unreachable!(),
    }
}

pub fn no_more<T: std::fmt::Debug>(rx: &Receiver<T>, within: Duration) {
    if let Ok(v) = rx.recv_timeout(within) {
        panic!("beklenmeyen: {v:?}");
    }
}

pub fn hints<'a>(pairs: impl IntoIterator<Item = (&'a str, Value<'a>)>) -> HashMap<&'a str, Value<'a>> {
    pairs.into_iter().collect()
}

pub fn svg(width: u32, height: u32) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}"><rect width="{width}" height="{height}"/></svg>"#
    )
}

pub fn pixmap_size(icon: &NotifyIcon) -> Option<(u32, u32, usize)> {
    match icon {
        NotifyIcon::Pixmap { width, height, rgba } => Some((*width, *height, rgba.len())),
        _ => None,
    }
}

/// `id`'nin `Added`'ını ve (görsel geciktiyse) ardından gelen `ImageReady`'leri uygular;
/// beklenen görseller hazır olunca bildirimin son halini döner.
pub fn settled(
    rx: &Receiver<Ev>,
    id: u32,
    want_app_icon: bool,
    want_image: bool,
) -> (nuxut_shell::services::notifications::Notification, usize) {
    let mut n = match wait_for(rx, "Added", |e| matches!(e, Ev::Added(n) if n.id == id)) {
        Ev::Added(n) => n,
        _ => unreachable!(),
    };
    let mut ready_events = 0;
    while (want_app_icon && n.app_icon.is_none()) || (want_image && n.image.is_none()) {
        match wait_for(rx, "ImageReady", |e| matches!(e, Ev::ImageReady { id: i, .. } if *i == id)) {
            Ev::ImageReady { app_icon, image, .. } => {
                ready_events += 1;
                if let Some(icon) = app_icon {
                    n.app_icon = icon;
                }
                if let Some(icon) = image {
                    n.image = icon;
                }
            }
            _ => unreachable!(),
        }
    }
    (n, ready_events)
}

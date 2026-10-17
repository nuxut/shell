//! Yerleşik ajan, özel bir `dbus-daemon` üzerinde sahte polkitd + logind ile. Diyalog yerine
//! POSIX sh betiği (çerezin önekine göre davranır). Kullanıcının bus'larına, gerçek polkitd'ye
//! ve hyprpolkitagent'a dokunulmaz.

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use nuxut_polkit::agent::process::DialogCommand;
use nuxut_polkit::agent::{self, AgentOptions};

const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const FAILED: &str = "org.freedesktop.PolicyKit1.Error.Failed";
const T: Duration = Duration::from_secs(10);

/// Sahte diyalog: isteği `$D/req.<n>`'e yazar, `$D/spawns`'a satır ekler; çerez önekine göre
/// `success`/`cancelled`/`failed`/`crash`/`hang` (SIGTERM'de `$D/term`).
const SCRIPT: &str = r#"D="$1"
read -r line
n=$(cat "$D/spawns" 2>/dev/null | wc -l)
printf '%s\n' "$line" > "$D/req.$n"
echo x >> "$D/spawns"
case "$line" in
  *'"cookie":"success'*) echo success ;;
  *'"cookie":"cancelled'*) echo cancelled ;;
  *'"cookie":"failed'*) echo failed ;;
  *'"cookie":"crash'*) kill -9 $$ ;;
  *'"cookie":"hang'*) trap 'echo term > "$D/term"; exit 0' TERM; while :; do sleep 0.05; done ;;
  *) echo garbage ;;
esac
"#;

struct Env {
    bus: PrivateBus,
    authority: zbus::blocking::Connection,
    state: std::sync::Arc<std::sync::Mutex<AuthorityState>>,
    _logind: zbus::blocking::Connection,
    dir: tempfile::TempDir,
}

impl Env {
    fn spawns(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("spawns"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    fn agent_name(&self) -> String {
        assert!(
            wait_until(T, || !self.state.lock().unwrap().registrations.is_empty()),
            "ajan kaydolmadı"
        );
        self.state.lock().unwrap().registrations.last().unwrap().sender.clone()
    }
}

fn no_backoff(_: u32) -> Duration {
    Duration::from_millis(100)
}

fn options(dir: &Path, env_session: Option<&str>) -> AgentOptions {
    let script = dir.join("dialog.sh");
    std::fs::write(&script, SCRIPT).unwrap();
    let mut o = AgentOptions::new(
        DialogCommand {
            program: PathBuf::from("/bin/sh"),
            args: vec![script.into(), dir.into()],
        },
        Duration::from_secs(300),
    );
    o.env_session = env_session.map(str::to_owned);
    o.locale = "tr_TR.UTF-8".into();
    o.backoff = no_backoff;
    o.kill_grace = Duration::from_millis(500);
    o
}

fn start_with(logind: LogindConfig, tweak: impl FnOnce(&mut AgentOptions), reject: u32) -> Option<Env> {
    start_with_state(logind, tweak, |s| s.reject_registrations = reject)
}

/// Sahte polkitd durumu ajan başlamadan `prepare` ile kurulur.
fn start_with_state(
    logind: LogindConfig,
    tweak: impl FnOnce(&mut AgentOptions),
    prepare: impl FnOnce(&mut AuthorityState),
) -> Option<Env> {
    let bus = PrivateBus::start()?;
    let (authority, state) = start_authority(&bus);
    prepare(&mut state.lock().unwrap());
    let logind_conn = start_logind(&bus, &logind, uid());
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options(dir.path(), None);
    tweak(&mut opts);
    let address = bus.address.clone();
    std::thread::spawn(move || {
        async_io::block_on(async move {
            let conn = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .build()
                .await
                .unwrap();
            let _ = agent::run(conn, opts).await;
        })
    });
    Some(Env {
        bus,
        authority,
        state,
        _logind: logind_conn,
        dir,
    })
}

fn start() -> Option<Env> {
    start_with(LogindConfig::simple(uid()), |_| {}, 0)
}

fn details_self() -> HashMap<&'static str, &'static str> {
    HashMap::new()
}

#[test]
fn registers_own_session_without_fallback() {
    let Some(env) = start() else { return };
    let name = env.agent_name();
    let reg = env.state.lock().unwrap().registrations[0].clone();
    assert!(name.starts_with(':'));
    assert_eq!(reg.kind, "unix-session");
    assert_eq!(reg.session_id.as_deref(), Some("c1"));
    assert_eq!(reg.path, AGENT_PATH);
    assert_eq!(reg.fallback, Some(false));
    assert_eq!(reg.locale, "tr_TR.UTF-8");
}

#[test]
fn outcomes_map_to_polkit_replies() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let me = std::process::id().to_string();
    let mut details = HashMap::new();
    details.insert("polkit.subject-pid", me.as_str());
    assert_eq!(begin(&env.authority, &agent, "success-1", uid(), details), Ok(()));
    let req = std::fs::read_to_string(env.dir.path().join("req.0")).unwrap();
    let req: serde_json::Value = serde_json::from_str(&req).unwrap();
    assert_eq!(req["cookie"], "success-1");
    assert_eq!(req["identities"], serde_json::json!([uid()]));
    assert_eq!(req["action_id"], "org.nuxut.test.action");
    assert_eq!(req["timeout_s"], 300);
    // Kendi sürecimizin exe yolu okunabilir (comm değil) ama test ikilisi kullanıcının
    // yazabildiği hedef dizininde: doğrulanmamış yol olarak gider.
    let exe = std::env::current_exe().unwrap();
    assert_eq!(req["requester"]["unverified_exe"].as_str(), exe.to_str(), "{req}");
    assert!(req["requester"].get("exe").is_none(), "{req}");
    assert!(req.get("password").is_none());
    assert_eq!(begin(&env.authority, &agent, "cancelled-2", uid(), details_self()), Err(CANCELLED.into()));
    assert_eq!(begin(&env.authority, &agent, "failed-3", uid(), details_self()), Err(FAILED.into()));
    // Diyalog çökerse/garip yazarsa yetki yok: Failed.
    assert_eq!(begin(&env.authority, &agent, "crash-4", uid(), details_self()), Err(FAILED.into()));
    assert_eq!(begin(&env.authority, &agent, "other-5", uid(), details_self()), Err(FAILED.into()));
    assert_eq!(env.spawns(), 5);
}

#[test]
fn foreign_sender_is_rejected_without_dialog() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let intruder = env.bus.connect();
    assert_eq!(begin(&intruder, &agent, "success-x", uid(), details_self()), Err(FAILED.into()));
    assert_eq!(cancel(&intruder, &agent, "success-x"), Err(FAILED.into()));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(env.spawns(), 0, "yabancı çağrı diyalog açmamalı");
    // Gerçek sahip çalışmaya devam eder.
    assert_eq!(begin(&env.authority, &agent, "success-z", uid(), details_self()), Ok(()));
}

#[test]
fn cancel_authentication_kills_dialog() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let conn = env.authority.clone();
    let a = agent.clone();
    let call = std::thread::spawn(move || begin(&conn, &a, "hang-1", uid(), HashMap::new()));
    assert!(wait_until(T, || env.spawns() == 1));
    assert_eq!(cancel(&env.authority, &agent, "nope"), Err(FAILED.into()));
    assert_eq!(cancel(&env.authority, &agent, "hang-1"), Ok(()));
    assert_eq!(call.join().unwrap(), Err(CANCELLED.into()));
    assert!(wait_until(T, || env.dir.path().join("term").exists()), "diyalog SIGTERM almadı");
    // Sonraki istek normal işlenir.
    assert_eq!(begin(&env.authority, &agent, "success-2", uid(), details_self()), Ok(()));
}

#[test]
fn queue_runs_one_dialog_at_a_time() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let (c1, a1) = (env.authority.clone(), agent.clone());
    let first = std::thread::spawn(move || begin(&c1, &a1, "hang-1", uid(), HashMap::new()));
    assert!(wait_until(T, || env.spawns() == 1));
    let (c2, a2) = (env.authority.clone(), agent.clone());
    let second = std::thread::spawn(move || begin(&c2, &a2, "success-2", uid(), HashMap::new()));
    let (c3, a3) = (env.authority.clone(), agent.clone());
    let third = std::thread::spawn(move || begin(&c3, &a3, "success-3", uid(), HashMap::new()));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(env.spawns(), 1, "etkin diyalog varken ikincisi açılmaz");
    // Bekleyen istek iptal edilebilir; diyalog hiç açılmaz.
    assert!(wait_until(T, || cancel(&env.authority, &agent, "success-3").is_ok()));
    assert_eq!(third.join().unwrap(), Err(CANCELLED.into()));
    assert_eq!(cancel(&env.authority, &agent, "hang-1"), Ok(()));
    assert_eq!(first.join().unwrap(), Err(CANCELLED.into()));
    assert_eq!(second.join().unwrap(), Ok(()));
    assert_eq!(env.spawns(), 2);
}

#[test]
fn hung_dialog_hits_backstop() {
    let Some(env) = start_with(
        LogindConfig::simple(uid()),
        |o| o.backstop = Duration::from_millis(500),
        0,
    ) else {
        return;
    };
    let agent = env.agent_name();
    assert_eq!(begin(&env.authority, &agent, "hang-1", uid(), details_self()), Err(CANCELLED.into()));
    assert!(wait_until(T, || env.dir.path().join("term").exists()));
}

#[test]
fn unsupported_identities_fail_without_dialog() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let mut m = HashMap::new();
    m.insert("gid", zbus::zvariant::Value::U32(10));
    let r = env
        .authority
        .call_method(
            Some(agent.as_str()),
            AGENT_PATH,
            Some(AGENT_IFACE),
            "BeginAuthentication",
            &("a", "m", "", HashMap::<&str, &str>::new(), "success-1", vec![("unix-group", m)]),
        )
        .map(|_| ())
        .map_err(error_name);
    assert_eq!(r, Err(FAILED.into()));
    assert_eq!(env.spawns(), 0);
}

#[test]
fn backs_off_while_another_agent_is_registered() {
    let Some(env) = start_with(LogindConfig::simple(uid()), |_| {}, 2) else { return };
    env.agent_name();
    let s = env.state.lock().unwrap();
    assert_eq!(s.attempts, 3, "iki ret + bir başarı");
    assert_eq!(s.registrations.len(), 1);
}

#[test]
fn reregisters_and_cancels_when_polkitd_restarts() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let (c1, a1) = (env.authority.clone(), agent.clone());
    let pending = std::thread::spawn(move || begin(&c1, &a1, "hang-1", uid(), HashMap::new()));
    assert!(wait_until(T, || env.spawns() == 1));
    // polkitd "yeniden başlar": ad bırakılır, yeni bir süreç (bağlantı) alır.
    env.authority.release_name("org.freedesktop.PolicyKit1").unwrap();
    assert_eq!(pending.join().unwrap(), Err(CANCELLED.into()));
    assert!(wait_until(T, || env.dir.path().join("term").exists()));
    let (new_authority, new_state) = start_authority(&env.bus);
    assert!(
        wait_until(T, || !new_state.lock().unwrap().registrations.is_empty()),
        "yeni polkitd'ye yeniden kayıt yok"
    );
    // Eski sahip artık çağıramaz; yenisi çağırabilir.
    assert_eq!(begin(&env.authority, &agent, "success-1", uid(), details_self()), Err(FAILED.into()));
    assert_eq!(begin(&new_authority, &agent, "success-2", uid(), details_self()), Ok(()));
}

/// polkitd kayıt sırasında adı bırakıp yeniden alır (etkinleştirme/yeniden başlama yarışı):
/// `sahip→""` ve `""→sahip` sinyalleri kayıt yanıtından önce gelir. Ajan aynı polkitd'ye
/// ikinci kez kaydolmaya çalışmamalı ("already exists" uyarı döngüsü yok); bekleyen istekler
/// yine de iptal edilir ve ajan çalışmaya devam eder.
#[test]
fn name_bounce_during_register_does_not_loop() {
    // Kayıt yanıtından önce ad sıçrar (sahte Authority bunu ilk kayıtta yapar; bayrak ajan
    // başlamadan kurulur).
    let Some(env) = start_with_state(LogindConfig::simple(uid()), |_| {}, |s| s.bounce_name_on_register = 1)
    else {
        return;
    };
    env.agent_name();
    // no_backoff 100 ms: döngü olsaydı 1,5 sn'de çok sayıda deneme birikirdi.
    std::thread::sleep(Duration::from_millis(1500));
    {
        let s = env.state.lock().unwrap();
        assert_eq!(s.attempts, 1, "aynı polkitd'ye ikinci kayıt denendi");
        assert_eq!(s.registrations.len(), 1);
    }
    // Ajan hâlâ aynı sahipten gelen istekleri işler.
    let agent = env.agent_name();
    assert_eq!(begin(&env.authority, &agent, "success-1", uid(), details_self()), Ok(()));
}

#[test]
fn session_fallbacks() {
    let me = uid();
    let other = me.wrapping_add(1);
    // PID oturumu yok → XDG_SESSION_ID (bize ait).
    let cfg = LogindConfig {
        by_pid: None,
        sessions: vec![("c5".into(), me), ("c9".into(), other)],
        display: Some("c7".into()),
    };
    let Some(env) = start_with(cfg.clone(), |o| o.env_session = Some("c5".into()), 0) else { return };
    env.agent_name();
    assert_eq!(env.state.lock().unwrap().registrations[0].session_id.as_deref(), Some("c5"));
    // XDG_SESSION_ID başkasının → Display oturumu.
    let Some(env) = start_with(cfg.clone(), |o| o.env_session = Some("c9".into()), 0) else { return };
    env.agent_name();
    assert_eq!(env.state.lock().unwrap().registrations[0].session_id.as_deref(), Some("c7"));
    // PID oturumu başkasının → kullanılmaz.
    let cfg2 = LogindConfig {
        by_pid: Some("c9".into()),
        ..cfg
    };
    let Some(env) = start_with(cfg2, |o| o.env_session = None, 0) else { return };
    env.agent_name();
    assert_eq!(env.state.lock().unwrap().registrations[0].session_id.as_deref(), Some("c7"));
}

#[test]
fn malformed_and_duplicate_cookies_rejected() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    let long = "x".repeat(300);
    for bad in ["", "a b", "success\nSUCCESS", "success\tx", long.as_str()] {
        assert_eq!(begin(&env.authority, &agent, bad, uid(), details_self()), Err(FAILED.into()), "{bad:?}");
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(env.spawns(), 0, "bozuk çerez diyalog açmamalı");
    // Etkin istekle aynı çerez: reddedilir, ikinci diyalog açılmaz.
    let (c, a) = (env.authority.clone(), agent.clone());
    let first = std::thread::spawn(move || begin(&c, &a, "hang-dup", uid(), HashMap::new()));
    assert!(wait_until(T, || env.spawns() == 1));
    assert_eq!(begin(&env.authority, &agent, "hang-dup", uid(), details_self()), Err(FAILED.into()));
    assert_eq!(cancel(&env.authority, &agent, "hang-dup"), Ok(()));
    assert_eq!(first.join().unwrap(), Err(CANCELLED.into()));
    assert_eq!(env.spawns(), 1);
}

/// İptal ile diyalog sonucu yarışır: her istek tam bir yanıt alır ve iptal yalnızca gerçekten
/// iptal ettiyse başarılı döner (iptal Ok ⇔ istek Cancelled). Başarı asla iptalle karışmaz.
#[test]
fn cancel_race_is_consistent() {
    let Some(env) = start() else { return };
    let agent = env.agent_name();
    for i in 0..20 {
        let cookie = format!("success-race-{i}");
        let (c, a, k) = (env.authority.clone(), agent.clone(), cookie.clone());
        let call = std::thread::spawn(move || begin(&c, &a, &k, uid(), HashMap::new()));
        std::thread::sleep(Duration::from_millis((i % 5) * 3));
        let cancelled = cancel(&env.authority, &agent, &cookie);
        let result = call.join().unwrap();
        match cancelled {
            Ok(()) => assert_eq!(result, Err(CANCELLED.into()), "tur {i}"),
            Err(e) => {
                assert_eq!(e, FAILED, "tur {i}");
                assert_eq!(result, Ok(()), "tur {i}");
            }
        }
    }
    // Ajan hâlâ çalışır.
    assert_eq!(begin(&env.authority, &agent, "success-after", uid(), details_self()), Ok(()));
}

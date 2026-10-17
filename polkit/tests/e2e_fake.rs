//! Uçtan uca (YALNIZCA `fake-polkit`): gerçek `nuxut-polkit` ajan süreci özel bus'ta sahte
//! polkitd + logind'e kaydolur, gerçek diyalog süreci (`dialog --headless`) sahte yardımcıyla
//! protokolü konuşur, sahte yardımcı başarıda sahte Authority'ye yanıt verir. Gerçek polkitd,
//! PAM ve kullanıcının oturumu hiç kullanılmaz.

mod common;

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::*;

const CANCELLED: &str = "org.freedesktop.PolicyKit1.Error.Cancelled";
const FAILED: &str = "org.freedesktop.PolicyKit1.Error.Failed";
const T: Duration = Duration::from_secs(15);
const SECRET: &str = "nuxut-test-sifresi";

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn helper(dir: &Path, bus: &PrivateBus, mode: &str) -> (Proc, std::path::PathBuf) {
    let sock = dir.join(format!("helper-{mode}.sock"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_nuxut-polkit-fake-helper"))
        .args(["--socket", sock.to_str().unwrap(), "--password", SECRET, "--bus", &bus.address, "--mode", mode])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready");
    (Proc(child), sock)
}

fn base_command(dir: &Path, sock: &Path, passwords: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_nuxut-polkit"));
    c.env("NUXUT_POLKIT_FAKE_HELPER_SOCKET", sock)
        .env("NUXUT_POLKIT_FAKE_HEADLESS", "1")
        .env("NUXUT_POLKIT_TEST_PASSWORDS", passwords)
        .env("XDG_CONFIG_HOME", dir)
        .env_remove("XDG_SESSION_ID")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("HYPRLAND_INSTANCE_SIGNATURE");
    c
}

fn agent(dir: &Path, bus: &PrivateBus, sock: &Path, passwords: &str) -> Proc {
    let child = base_command(dir, sock, passwords)
        .env("NUXUT_POLKIT_FAKE_BUS", &bus.address)
        .spawn()
        .unwrap();
    Proc(child)
}

struct Env {
    bus: PrivateBus,
    authority: zbus::blocking::Connection,
    state: std::sync::Arc<std::sync::Mutex<AuthorityState>>,
    _logind: zbus::blocking::Connection,
    dir: tempfile::TempDir,
}

fn env() -> Option<Env> {
    let bus = PrivateBus::start()?;
    let (authority, state) = start_authority(&bus);
    let logind = start_logind(&bus, &LogindConfig::simple(uid()), uid());
    Some(Env {
        bus,
        authority,
        state,
        _logind: logind,
        dir: tempfile::tempdir().unwrap(),
    })
}

impl Env {
    fn agent_name(&self) -> String {
        assert!(wait_until(T, || !self.state.lock().unwrap().registrations.is_empty()), "kayıt yok");
        self.state.lock().unwrap().registrations[0].sender.clone()
    }

    fn responses(&self) -> Vec<(u32, String, u32)> {
        self.state.lock().unwrap().responses.clone()
    }
}

/// Bir sürecin çocukları (tüm thread'leri).
fn children(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for t in tasks.flatten() {
            if let Ok(s) = std::fs::read_to_string(t.path().join("children")) {
                out.extend(s.split_whitespace().filter_map(|p| p.parse::<u32>().ok()));
            }
        }
    }
    out
}

fn alive(pid: u32) -> bool {
    // Zombi de "ölü" sayılır.
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|s| !s.rsplit(')').next().unwrap_or("").trim_start().starts_with('Z'))
        .unwrap_or(false)
}

fn kill9(pid: u32) {
    let pid = rustix::process::Pid::from_raw(pid as i32).unwrap();
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
}

fn begin_async(env: &Env, agent: &str, cookie: &str) -> std::thread::JoinHandle<Result<(), String>> {
    let (c, a, k) = (env.authority.clone(), agent.to_owned(), cookie.to_owned());
    std::thread::spawn(move || begin(&c, &a, &k, uid(), HashMap::new()))
}

#[test]
fn success_reaches_authority() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "normal");
    let _a = agent(env.dir.path(), &env.bus, &sock, SECRET);
    let name = env.agent_name();
    assert_eq!(begin(&env.authority, &name, "3-ok-1", uid(), HashMap::new()), Ok(()));
    assert_eq!(env.responses(), [(uid(), "3-ok-1".to_owned(), uid())]);
}

#[test]
fn wrong_then_right_password() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "info");
    let _a = agent(env.dir.path(), &env.bus, &sock, &format!("yanlis,{SECRET}"));
    let name = env.agent_name();
    assert_eq!(begin(&env.authority, &name, "3-retry-1", uid(), HashMap::new()), Ok(()));
    assert_eq!(env.responses().len(), 1, "yalnızca doğru parola yanıt üretir");
}

#[test]
fn wrong_password_then_polkit_cancel() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "normal");
    let a = agent(env.dir.path(), &env.bus, &sock, "yanlis");
    let name = env.agent_name();
    let call = begin_async(&env, &name, "3-cancel-1");
    let agent_pid = a.0.id();
    assert!(wait_until(T, || !children(agent_pid).is_empty()));
    let dialog = children(agent_pid)[0];
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(cancel(&env.authority, &name, "3-cancel-1"), Ok(()));
    assert_eq!(call.join().unwrap(), Err(CANCELLED.into()));
    assert!(wait_until(T, || !alive(dialog)), "diyalog iptalde kapanmadı");
    assert!(env.responses().is_empty());
}

#[test]
fn killed_dialog_fails_closed() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "normal");
    let a = agent(env.dir.path(), &env.bus, &sock, "");
    let name = env.agent_name();
    let call = begin_async(&env, &name, "3-kill-1");
    assert!(wait_until(T, || !children(a.0.id()).is_empty()));
    kill9(children(a.0.id())[0]);
    assert_eq!(call.join().unwrap(), Err(FAILED.into()));
    assert!(env.responses().is_empty());
}

#[test]
fn killed_agent_takes_dialog_down() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "normal");
    let a = agent(env.dir.path(), &env.bus, &sock, "");
    let name = env.agent_name();
    let call = begin_async(&env, &name, "3-agent-1");
    assert!(wait_until(T, || !children(a.0.id()).is_empty()));
    let dialog = children(a.0.id())[0];
    kill9(a.0.id());
    assert!(wait_until(T, || !alive(dialog)), "ajan ölünce diyalog kapanmadı");
    assert!(call.join().unwrap().is_err());
    assert!(env.responses().is_empty());
}

#[test]
fn garbage_and_long_helper_lines_fail() {
    let Some(env) = env() else { return };
    for mode in ["garbage", "long", "eof"] {
        let (_h, sock) = helper(env.dir.path(), &env.bus, mode);
        let a = agent(env.dir.path(), &env.bus, &sock, SECRET);
        let name = {
            assert!(wait_until(T, || env.state.lock().unwrap().registrations.len() == 1));
            env.state.lock().unwrap().registrations.remove(0).sender
        };
        assert_eq!(
            begin(&env.authority, &name, &format!("3-{mode}"), uid(), HashMap::new()),
            Err(FAILED.into()),
            "{mode}"
        );
        drop(a);
        // Ad bırakılsın (aynı bus'ta sonraki ajan).
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(env.responses().is_empty());
}

fn run_dialog(dir: &Path, sock: &Path, passwords: &str, request: &str) -> (String, Duration) {
    let start = std::time::Instant::now();
    let mut child = base_command(dir, sock, passwords)
        .args(["dialog", "--headless"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(request.as_bytes()).unwrap();
    // stdin kapanır: ajan gitti sayılır.
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    (String::from_utf8(out.stdout).unwrap(), start.elapsed())
}

fn request_json(timeout_s: u64, extra: &str) -> String {
    format!(
        "{{\"cookie\":\"c-1\",\"action_id\":\"a\",\"message\":\"m\",\"icon_name\":\"\",\"identities\":[{}],\"requester\":null,\"timeout_s\":{timeout_s}{extra}}}\n",
        uid()
    )
}

#[test]
fn dialog_contract_and_inactivity_timeout() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "normal");
    // Parola alanı içeren istek reddedilir (yardımcıya hiç bağlanılmaz).
    let (out, _) = run_dialog(env.dir.path(), &sock, SECRET, &request_json(5, ",\"password\":\"x\""));
    assert_eq!(out, "failed\n");
    // Etkileşimsiz süre dolunca iptal (stdin açık tutulur: ajan yaşıyor).
    let start = std::time::Instant::now();
    let mut child = base_command(env.dir.path(), &sock, "")
        .args(["dialog", "--headless"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(request_json(1, "").as_bytes()).unwrap();
    let mut out = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut out).unwrap();
    assert_eq!(out, "cancelled\n");
    assert!(start.elapsed() >= Duration::from_millis(900));
    drop(stdin);
    let _ = child.wait();
    // stdin EOF (ajan gitti) → iptal.
    let (out, took) = run_dialog(env.dir.path(), &sock, "", &request_json(60, ""));
    assert_eq!(out, "cancelled\n");
    assert!(took < Duration::from_secs(10));
    // Sahte olmayan yardımcı yoluyla başsız diyalog çalışmaz (gerçek polkit'e asla gitmez).
    let (out, _) = run_dialog(env.dir.path(), Path::new("/run/polkit/agent-helper.socket"), SECRET, &request_json(5, ""));
    assert_eq!(out, "failed\n");
}

/// Parola hiçbir loga girmez: ajan + diyalog stderr'i (en ayrıntılı düzeyde) yanlış ve doğru
/// parolayı içermez. Diyalog stderr'i ajanınkini devralır; ikisi de aynı dosyaya düşer.
#[test]
fn secret_never_logged() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "info");
    let wrong = "yanlis-parola-7731";
    let log = env.dir.path().join("agent.log");
    let file = std::fs::File::create(&log).unwrap();
    let child = base_command(env.dir.path(), &sock, &format!("{wrong},{SECRET}"))
        .env("NUXUT_POLKIT_FAKE_BUS", &env.bus.address)
        .env("RUST_LOG", "trace")
        .stderr(Stdio::from(file))
        .spawn()
        .unwrap();
    let a = Proc(child);
    let name = env.agent_name();
    assert_eq!(begin(&env.authority, &name, "3-log-1", uid(), HashMap::new()), Ok(()));
    drop(a);
    let text = std::fs::read(&log).unwrap();
    assert!(!text.is_empty(), "log boş: test anlamsız olurdu");
    for needle in [SECRET, wrong] {
        assert!(
            !text.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "logda parola var: {needle}"
        );
    }
}

/// Yarıda kesilen yardımcı (satır sonusuz `SUCC` + kapanış) asla başarı sayılmaz: parola
/// gönderilmişse yanlış deneme gibi yeniden denenir; iptalde `Cancelled`, yetki yanıtı yok.
#[test]
fn interrupted_helper_is_never_success() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "partial");
    let a = agent(env.dir.path(), &env.bus, &sock, &format!("{SECRET},{SECRET}"));
    let name = env.agent_name();
    let call = begin_async(&env, &name, "3-partial-1");
    assert!(wait_until(T, || !children(a.0.id()).is_empty()));
    let dialog = children(a.0.id())[0];
    std::thread::sleep(Duration::from_millis(1000));
    assert!(!call.is_finished(), "yarım SUCCESS isteği bitirmemeli");
    assert_eq!(cancel(&env.authority, &name, "3-partial-1"), Ok(()));
    assert_eq!(call.join().unwrap(), Err(CANCELLED.into()));
    assert!(wait_until(T, || !alive(dialog)));
    assert!(env.responses().is_empty());
}

/// İki eşzamanlı istek: gerçek diyalog süreçleri sırayla açılır (aynı anda en çok bir), ikisi de
/// kendi çereziyle doğrulanır.
#[test]
fn two_concurrent_requests_run_in_order() {
    let Some(env) = env() else { return };
    let (_h, sock) = helper(env.dir.path(), &env.bus, "normal");
    let a = agent(env.dir.path(), &env.bus, &sock, SECRET);
    let name = env.agent_name();
    let first = begin_async(&env, &name, "3-pair-1");
    let second = begin_async(&env, &name, "3-pair-2");
    let pid = a.0.id();
    let mut max_children = 0;
    while !(first.is_finished() && second.is_finished()) {
        max_children = max_children.max(children(pid).iter().filter(|c| alive(**c)).count());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(first.join().unwrap(), Ok(()));
    assert_eq!(second.join().unwrap(), Ok(()));
    assert!(max_children <= 1, "aynı anda {max_children} diyalog");
    let mut cookies: Vec<String> = env.responses().into_iter().map(|(_, c, _)| c).collect();
    cookies.sort();
    assert_eq!(cookies, ["3-pair-1", "3-pair-2"]);
}

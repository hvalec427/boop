//! WebDriverAgent, the XCTest runner that drives iOS UI (what Appium and
//! Maestro use underneath). touchctl builds it once per Xcode version into
//! `~/.cache/touchctl`, and runs one per simulator in the background, on its
//! own port. `~/.cache/touchctl/wda/<udid>.json` records the pid and port, so
//! later commands reuse it.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// WebDriverAgent release to build (override with TOUCHCTL_WDA_TAG). Newer
/// ones embed `lib_TestingInterop.dylib`, which Xcode 26.2 doesn't ship.
pub const WDA_TAG: &str = "v16.12.10";
const REPO: &str = "https://github.com/appium/WebDriverAgent";

#[derive(Serialize, Deserialize)]
struct Running {
    pid: u32,
    port: u16,
}

fn cache() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::env::var("TOUCHCTL_CACHE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(home).join(".cache/touchctl"))
}

fn tag() -> String {
    std::env::var("TOUCHCTL_WDA_TAG").unwrap_or_else(|_| WDA_TAG.into())
}

fn entry(udid: &str) -> PathBuf {
    cache().join("wda").join(format!("{udid}.json"))
}

pub fn log_path(udid: &str) -> PathBuf {
    cache().join("wda").join(format!("{udid}.log"))
}

fn pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn alive(port: u16) -> bool {
    reqwest::blocking::Client::new().get(format!("http://127.0.0.1:{port}/status")).timeout(Duration::from_secs(2)).send().is_ok_and(|r| r.status().is_success())
}

fn running(udid: &str) -> Option<Running> {
    let r: Running = serde_json::from_slice(&std::fs::read(entry(udid)).ok()?).ok()?;
    if pid_alive(r.pid) {
        Some(r)
    } else {
        let _ = std::fs::remove_file(entry(udid));
        None
    }
}

/// Port of the WebDriverAgent serving `udid`, if one is up.
pub fn status(udid: &str) -> Option<u16> {
    running(udid).map(|r| r.port).filter(|p| alive(*p))
}

/// Ports in use by other WebDriverAgents (or anything listening).
fn free_port() -> Result<u16> {
    let taken: Vec<u16> = std::fs::read_dir(cache().join("wda"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice::<Running>(&std::fs::read(e.path()).ok()?).ok())
        .filter(|r| pid_alive(r.pid))
        .map(|r| r.port)
        .collect();
    (8100..8300)
        .find(|p| !taken.contains(p) && std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], *p).into(), Duration::from_millis(100)).is_err())
        .ok_or_else(|| anyhow!("no free port in 8100–8299"))
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let so = String::from_utf8_lossy(&out.stdout);
        let text = if err.trim().is_empty() { so } else { err };
        let lines: Vec<&str> = text.trim_end().lines().collect();
        bail!("{}", lines[lines.len().saturating_sub(15)..].join("\n"));
    }
    Ok(())
}

/// Held while cloning/building, so parallel touchctls don't build into the
/// same directory. A lock older than 20 minutes is left over from a crash.
struct BuildLock(PathBuf);

impl BuildLock {
    fn take() -> Result<BuildLock> {
        let p = cache().join("wda-build.lock");
        std::fs::create_dir_all(cache())?;
        let start = Instant::now();
        loop {
            if std::fs::create_dir(&p).is_ok() {
                return Ok(BuildLock(p));
            }
            let stale = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(20 * 60));
            if stale {
                let _ = std::fs::remove_dir(&p);
                continue;
            }
            if start.elapsed().as_secs() % 30 == 0 {
                eprintln!("waiting for another WebDriverAgent build…");
            }
            if start.elapsed() > Duration::from_secs(20 * 60) {
                bail!("timed out waiting for {}", p.display());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// The built `.xctestrun` for simulators, building it first if needed.
fn xctestrun() -> Result<PathBuf> {
    let xcode = String::from_utf8_lossy(&Command::new("xcodebuild").arg("-version").output().context("xcodebuild not found (install Xcode)")?.stdout)
        .lines()
        .next()
        .unwrap_or("Xcode")
        .replace(' ', "");
    let tag = tag();
    let build = cache().join(format!("wda-build-{tag}-{xcode}"));
    let find = || -> Option<PathBuf> {
        std::fs::read_dir(build.join("Build/Products")).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "xctestrun"))
    };
    if let Some(p) = find() {
        return Ok(p);
    }
    let _lock = BuildLock::take()?;
    if let Some(p) = find() {
        return Ok(p); // built while we waited
    }
    let src = cache().join("WebDriverAgent");
    if !src.join(".git").exists() {
        eprintln!("cloning WebDriverAgent…");
        run(Command::new("git").args(["clone", "-q", REPO]).arg(&src))?;
    }
    let has_tag = Command::new("git").arg("-C").arg(&src).args(["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")]).output()?.status.success();
    if !has_tag {
        run(Command::new("git").arg("-C").arg(&src).args(["fetch", "-q", "--tags"]))?;
    }
    run(Command::new("git").arg("-C").arg(&src).args(["checkout", "-q", &tag]))?;
    eprintln!("building WebDriverAgent {tag} for {xcode} (once, a minute or two)…");
    run(Command::new("xcodebuild")
        .args(["build-for-testing", "-project", "WebDriverAgent.xcodeproj", "-scheme", "WebDriverAgentRunner", "-destination", "generic/platform=iOS Simulator", "-derivedDataPath"])
        .arg(&build)
        .args(["CODE_SIGNING_ALLOWED=NO", "-quiet"])
        .current_dir(&src))
    .context("building WebDriverAgent (set TOUCHCTL_WDA_TAG to try another release)")?;
    find().ok_or_else(|| anyhow!("WebDriverAgent built, but no .xctestrun in {}", build.display()))
}

/// Port of a running WebDriverAgent for the simulator `udid`, starting it
/// (and building it, the first time) if needed.
pub fn ensure(udid: &str) -> Result<u16> {
    if let Some(r) = running(udid) {
        if wait_up(r.port, Duration::from_secs(60)) {
            return Ok(r.port);
        }
        stop(udid); // up but not answering
    }
    let testrun = xctestrun()?;
    let port = free_port()?;
    std::fs::create_dir_all(cache().join("wda"))?;
    let log = std::fs::File::create(log_path(udid))?;
    let mut cmd = Command::new("xcodebuild");
    cmd.args(["test-without-building", "-xctestrun"]).arg(&testrun).args(["-destination", &format!("id={udid}")]);
    cmd.env("TEST_RUNNER_USE_PORT", port.to_string()).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    // Its own session: it outlives this command, like a daemon.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let child = cmd.spawn().context("starting WebDriverAgent")?;
    std::fs::write(entry(udid), serde_json::to_vec(&Running { pid: child.id(), port })?)?;
    eprintln!("starting WebDriverAgent on :{port}…");
    if wait_up(port, Duration::from_secs(180)) {
        return Ok(port);
    }
    let tail = std::fs::read_to_string(log_path(udid)).unwrap_or_default();
    let lines: Vec<&str> = tail.lines().collect();
    stop(udid);
    bail!("WebDriverAgent didn't come up on :{port}. Last output:\n{}", lines[lines.len().saturating_sub(15)..].join("\n"))
}

fn wait_up(port: u16, max: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < max {
        if alive(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// Stop the WebDriverAgent serving `udid`. Returns whether one was running.
pub fn stop(udid: &str) -> bool {
    let Some(r) = running(udid) else {
        return false;
    };
    unsafe {
        // The whole session: xcodebuild and the runner it launched.
        libc::kill(-(r.pid as i32), libc::SIGTERM);
        libc::kill(r.pid as i32, libc::SIGTERM);
    }
    let _ = std::fs::remove_file(entry(udid));
    true
}

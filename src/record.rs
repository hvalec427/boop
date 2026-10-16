//! `boop record start` / `boop record stop`: record the device's screen to an
//! .mp4. The recorder runs in the background between the two commands;
//! `~/.cache/boop/record/<device>.json` remembers it, so one recording per
//! device runs at a time.

use crate::device::Device;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where Android records before `stop` pulls the file.
const ANDROID_FILE: &str = "/sdcard/boop-record.mp4";

#[derive(Serialize, Deserialize)]
struct Recording {
    pid: u32,
    /// The file being written (iOS: on this Mac; Android: pulled on stop).
    file: PathBuf,
}

fn state(d: &Device) -> PathBuf {
    let id = match d {
        Device::IosSim { udid, .. } => udid.clone(),
        Device::Android { serial } => serial.replace([':', '/'], "_"),
    };
    crate::wda::cache().join("record").join(format!("{id}.json"))
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn start(d: &Device) -> Result<()> {
    let st = state(d);
    if let Some(r) = std::fs::read(&st).ok().and_then(|b| serde_json::from_slice::<Recording>(&b).ok()) {
        if alive(r.pid) {
            bail!("already recording {}; boop record stop first", d.describe());
        }
    }
    std::fs::create_dir_all(st.parent().unwrap())?;
    let file = std::env::temp_dir().join(format!("boop-record-{}.mp4", std::process::id()));
    let mut cmd = match d {
        Device::IosSim { udid, .. } => {
            let mut c = Command::new("xcrun");
            c.args(["simctl", "io", udid, "recordVideo", "--codec=h264", "--force"]).arg(&file);
            c
        }
        Device::Android { serial } => {
            let mut c = Command::new("adb");
            c.args(["-s", serial, "shell", "screenrecord", ANDROID_FILE]);
            c
        }
    };
    // Its own process group: it outlives this command, and stop signals it alone.
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0).spawn().context("starting the recorder")?;
    std::thread::sleep(Duration::from_millis(800));
    if !alive(child.id()) {
        bail!("the recorder stopped right away (is {} running?)", d.describe());
    }
    std::fs::write(&st, serde_json::to_vec(&Recording { pid: child.id(), file })?)?;
    println!("recording {}; boop record stop <file.mp4> to finish", d.describe());
    Ok(())
}

/// Stop the recording and save it to `out` (default: a temp file); returns the path.
pub fn stop(d: &Device, out: Option<&Path>) -> Result<PathBuf> {
    let st = state(d);
    let r: Recording = std::fs::read(&st).ok().and_then(|b| serde_json::from_slice(&b).ok()).with_context(|| format!("not recording {}", d.describe()))?;
    let _ = std::fs::remove_file(&st);
    if let Device::Android { serial } = d {
        // screenrecord finishes its file on SIGINT, inside the device.
        let _ = Command::new("adb").args(["-s", serial, "shell", "pkill", "-INT", "screenrecord"]).output();
    }
    // SIGINT lets simctl finish the file; give it a moment, then insist.
    unsafe { libc::kill(r.pid as i32, libc::SIGINT) };
    let t = Instant::now();
    while alive(r.pid) && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(100));
    }
    if alive(r.pid) {
        unsafe { libc::kill(r.pid as i32, libc::SIGKILL) };
    }
    let out = out.map(Path::to_path_buf).unwrap_or_else(|| std::env::temp_dir().join(format!("boop-{}.mp4", std::process::id())));
    match d {
        Device::IosSim { .. } => {
            if !r.file.exists() {
                bail!("nothing was recorded");
            }
            if std::fs::rename(&r.file, &out).is_err() {
                std::fs::copy(&r.file, &out)?;
                let _ = std::fs::remove_file(&r.file);
            }
        }
        Device::Android { serial } => {
            std::thread::sleep(Duration::from_millis(500));
            let pulled = Command::new("adb").args(["-s", serial, "pull", ANDROID_FILE]).arg(&out).output()?;
            let _ = Command::new("adb").args(["-s", serial, "shell", "rm", "-f", ANDROID_FILE]).output();
            if !pulled.status.success() {
                bail!("pulling the recording failed: {}", String::from_utf8_lossy(&pulled.stderr).trim());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_file_per_device() {
        let ios = Device::IosSim { udid: "U-1".into(), name: "iPhone".into() };
        let android = Device::Android { serial: "192.168.0.2:5555".into() };
        assert!(state(&ios).ends_with("record/U-1.json"));
        assert!(state(&android).ends_with("record/192.168.0.2_5555.json"));
    }
}

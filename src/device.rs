//! Which device a command acts on, and the per-platform pieces that aren't
//! UI-tree driven: screenshots and opening URLs.

use crate::ui::Backend;
use crate::wda;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub enum Device {
    IosSim { udid: String, name: String },
    Android { serial: String },
}

impl Device {
    pub fn describe(&self) -> String {
        match self {
            Device::IosSim { udid, name } => format!("{name} ({udid}, iOS simulator)"),
            Device::Android { serial } => format!("{serial} (Android)"),
        }
    }

    /// The UI backend, starting WebDriverAgent on iOS if needed.
    pub fn backend(&self) -> Result<Backend> {
        Ok(match self {
            Device::IosSim { udid, .. } => Backend::Wda { port: wda::ensure(udid)? },
            Device::Android { serial } => Backend::Adb { serial: serial.clone() },
        })
    }

    /// Screenshot as JPEG at `path`, scaled to at most `max` px tall (0 = full size).
    pub fn screenshot(&self, path: &Path, max: u32) -> Result<()> {
        match self {
            Device::IosSim { udid, .. } => {
                let out = Command::new("xcrun").args(["simctl", "io", udid, "screenshot", "--type=jpeg"]).arg(path).output()?;
                if !out.status.success() {
                    bail!("screenshot failed: {}", String::from_utf8_lossy(&out.stderr).trim());
                }
            }
            Device::Android { serial } => {
                let out = Command::new("adb").args(["-s", serial, "exec-out", "screencap", "-p"]).output()?;
                if !out.status.success() || out.stdout.is_empty() {
                    bail!("screenshot failed: {}", String::from_utf8_lossy(&out.stderr).trim());
                }
                let png = path.with_extension("png");
                std::fs::write(&png, &out.stdout)?;
                let _ = Command::new("sips").args(["-s", "format", "jpeg"]).arg(&png).arg("--out").arg(path).output();
                let _ = std::fs::remove_file(&png);
            }
        }
        if max > 0 {
            let _ = Command::new("sips").args(["-Z", &max.to_string(), "-s", "formatOptions", "70"]).arg(path).output();
        }
        Ok(())
    }

    pub fn open_url(&self, url: &str) -> Result<()> {
        let out = match self {
            Device::IosSim { udid, .. } => Command::new("xcrun").args(["simctl", "openurl", udid, url]).output()?,
            Device::Android { serial } => Command::new("adb").args(["-s", serial, "shell", "am", "start", "-a", "android.intent.action.VIEW", "-d", url]).output()?,
        };
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }
}

/// Booted iOS simulators: (udid, name).
pub fn booted_sims() -> Vec<(String, String)> {
    let Ok(out) = Command::new("xcrun").args(["simctl", "list", "devices", "booted", "-j"]).output() else {
        return Vec::new();
    };
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    v["devices"]
        .as_object()
        .into_iter()
        .flat_map(|o| o.values())
        .flat_map(|l| l.as_array().into_iter().flatten())
        .filter(|d| d["state"] == "Booted")
        .map(|d| (d["udid"].as_str().unwrap_or("").to_string(), d["name"].as_str().unwrap_or("").to_string()))
        .collect()
}

/// Ready Android devices and emulators (adb serials).
pub fn android_serials() -> Vec<String> {
    let Ok(out) = Command::new("adb").arg("devices").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout).lines().skip(1).filter_map(|l| l.split_once('\t')).filter(|(_, s)| s.trim() == "device").map(|(d, _)| d.to_string()).collect()
}

/// An Android device's `--device` id and name: an emulator is known by its
/// AVD name (its serial changes every boot), a phone by its serial and model.
pub fn android_id(serial: &str) -> (String, String) {
    let adb = |args: &[&str]| Command::new("adb").arg("-s").arg(serial).args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").trim().to_string()).filter(|s| !s.is_empty());
    if serial.starts_with("emulator-") {
        if let Some(avd) = adb(&["emu", "avd", "name"]) {
            return (avd.clone(), avd);
        }
    }
    (serial.to_string(), adb(&["shell", "getprop", "ro.product.model"]).unwrap_or_default())
}

/// Every device touchctl can drive, with its id and name.
fn all_devices() -> Vec<(String, String, Device)> {
    let mut all: Vec<(String, String, Device)> = booted_sims().into_iter().map(|(udid, name)| (udid.clone(), name.clone(), Device::IosSim { udid, name })).collect();
    for serial in android_serials() {
        let (id, name) = android_id(&serial);
        all.push((id, name, Device::Android { serial }));
    }
    all
}

/// A `--device` value → Device, resolved like simon and metroctl do: an iOS
/// udid, an AVD name, an adb serial, or a name only one device has.
pub fn by_id(id: &str) -> Result<Device> {
    if let Some(name) = sim_name(id) {
        return Ok(Device::IosSim { udid: id.to_string(), name });
    }
    if android_serials().iter().any(|s| s == id) {
        return Ok(Device::Android { serial: id.to_string() });
    }
    let all = all_devices();
    if let Some((_, _, d)) = all.iter().find(|(i, _, _)| i == id) {
        return Ok(d.clone());
    }
    let named: Vec<&(String, String, Device)> = all.iter().filter(|(_, n, _)| n == id).collect();
    match named.len() {
        1 => return Ok(named[0].2.clone()),
        0 => {}
        _ => bail!("several devices are named {id}; pass an id: {}", named.iter().map(|(i, _, _)| i.as_str()).collect::<Vec<_>>().join(", ")),
    }
    if id.len() >= 24 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        bail!("{id} isn't a simulator or adb device; physical iPhones aren't supported yet");
    }
    bail!("no running device {id} (see `touchctl devices`)")
}

fn sim_name(udid: &str) -> Option<String> {
    let out = Command::new("xcrun").args(["simctl", "list", "devices", "-j"]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v["devices"].as_object()?.values().flat_map(|l| l.as_array().into_iter().flatten()).find(|d| d["udid"] == udid).map(|d| d["name"].as_str().unwrap_or("").to_string())
}

/// The device id in a JSON file's `udid` field, e.g. metroctl's
/// `.metroctl/session.json`, looked up from `path` or its parent dirs when
/// relative.
fn id_from_file(path: &Path) -> Result<String> {
    let file = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let cwd = std::env::current_dir()?;
        cwd.ancestors().map(|d| d.join(path)).find(|p| p.exists()).unwrap_or_else(|| cwd.join(path))
    };
    let v: Value = serde_json::from_slice(&std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?)?;
    v["udid"].as_str().map(String::from).ok_or_else(|| anyhow!("{} has no device yet (no \"udid\")", file.display()))
}

/// `--device`, else $TOUCHCTL_DEVICE, else `--device-from`/$TOUCHCTL_DEVICE_FROM,
/// else the only booted device.
pub fn resolve(device: Option<&str>, from: Option<&PathBuf>) -> Result<Device> {
    if let Some(d) = device.map(String::from).or_else(|| std::env::var("TOUCHCTL_DEVICE").ok().filter(|d| !d.is_empty())) {
        return by_id(&d);
    }
    if let Some(f) = from.cloned().or_else(|| std::env::var("TOUCHCTL_DEVICE_FROM").ok().filter(|f| !f.is_empty()).map(PathBuf::from)) {
        return by_id(&id_from_file(&f)?);
    }
    let mut all: Vec<Device> = all_devices().into_iter().map(|(_, _, d)| d).collect();
    match all.len() {
        1 => Ok(all.remove(0)),
        0 => bail!("no booted simulator or adb device"),
        _ => bail!("several devices are up; pick one with --device:\n{}", all.iter().map(|d| format!("  {}", d.describe())).collect::<Vec<_>>().join("\n")),
    }
}

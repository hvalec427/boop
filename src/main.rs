//! boop — control iOS simulators and Android devices from the command
//! line: read the element tree, tap, swipe, type, press buttons, take
//! screenshots. Built for coding agents (which call it through their shell)
//! as much as for people and scripts.

mod device;
mod ui;
mod update;
mod wda;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "boop", version, about = "Control iOS simulators and Android devices: element tree, tap, swipe, type, screenshot")]
struct Cli {
    /// iOS udid, AVD name, adb serial, or a name only one device has (default: $BOOP_DEVICE, else the only booted device)
    #[arg(long, global = true)]
    device: Option<String>,
    /// Read the device from a JSON file's "udid" (e.g. .signalbox/session.json; also $BOOP_DEVICE_FROM)
    #[arg(long = "device-from", global = true)]
    device_from: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List booted simulators and adb devices
    Devices,
    /// Elements on screen: type, #id, "label", @x,y (the point `tap` uses)
    Ui {
        /// Only elements whose id or label contains this
        filter: Option<String>,
        /// Print JSON
        #[arg(long)]
        json: bool,
    },
    /// Tap an element by id (preferred) or label, or a point: `tap 120 300`
    Tap {
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
        x: Option<i64>,
        y: Option<i64>,
    },
    /// Swipe: up (content scrolls down), down, left, right; or --from X,Y --to X,Y
    Swipe {
        direction: Option<String>,
        #[arg(long, value_parser = point)]
        from: Option<(i64, i64)>,
        #[arg(long, value_parser = point)]
        to: Option<(i64, i64)>,
        /// Duration in ms
        #[arg(long, default_value_t = 300)]
        ms: u64,
    },
    /// Type text into the focused field, or the one with --id/--label (tapped first)
    Type {
        text: String,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        label: Option<String>,
    },
    /// Press a button: home, back (Android), enter
    Press { button: String },
    /// Save a screenshot (JPEG) and print its path
    Screenshot {
        /// Output file (default: a temp file)
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Scale to at most this many px tall (0 = full size)
        #[arg(long, default_value_t = 1000)]
        max: u32,
    },
    /// Open a URL or deep link
    Open { url: String },
    /// Manage the WebDriverAgent that drives an iOS simulator
    Wda {
        #[command(subcommand)]
        action: WdaAction,
    },
    /// Check whether a newer version of boop is available
    #[command(name = "check-update")]
    CheckUpdate {
        #[arg(long)]
        stable: bool,
        #[arg(long)]
        nightly: bool,
        #[arg(long)]
        dev: bool,
    },
    /// Update boop to the latest version (stays on the installed build's channel)
    Update {
        #[arg(long)]
        stable: bool,
        #[arg(long)]
        nightly: bool,
        #[arg(long)]
        dev: bool,
        /// Install the channel's latest even if it's the same or an older version
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum WdaAction {
    /// Start it (building it first if needed); prints the port
    Start,
    /// Stop it
    Stop,
    /// Print its port if it's running
    Status,
}

fn point(s: &str) -> Result<(i64, i64), String> {
    let (x, y) = s.split_once(',').ok_or("expected X,Y")?;
    Ok((x.trim().parse().map_err(|_| "bad X")?, y.trim().parse().map_err(|_| "bad Y")?))
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let dev = || device::resolve(cli.device.as_deref(), cli.device_from.as_ref());
    match cli.command {
        Command::Devices => {
            for (udid, name) in device::booted_sims() {
                println!("{udid}  {name} (iOS simulator)");
            }
            for s in device::android_serials() {
                let (id, name) = device::android_id(&s);
                let kind = if s.starts_with("emulator-") { format!("Android emulator, {s}") } else { "Android".into() };
                println!("{id}  {name} ({kind})");
            }
        }
        Command::Ui { filter, json } => {
            let els = dev()?.backend()?.elements()?;
            let f = filter.map(|f| f.to_lowercase());
            let els: Vec<&ui::Element> = els.iter().filter(|e| f.as_ref().is_none_or(|f| e.id.as_ref().is_some_and(|i| i.to_lowercase().contains(f)) || e.label.as_ref().is_some_and(|l| l.to_lowercase().contains(f)))).collect();
            if json {
                let v: Vec<serde_json::Value> = els.iter().map(|e| serde_json::json!({ "type": e.kind, "id": e.id, "label": e.label, "value": e.value, "x": e.x, "y": e.y, "width": e.w, "height": e.h, "center": e.center() })).collect();
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else if els.is_empty() {
                println!("(no matching elements)");
            } else {
                for e in els {
                    println!("{}", e.line());
                }
            }
        }
        Command::Tap { id, label, x, y } => {
            let b = dev()?.backend()?;
            let (x, y, what) = match (x, y) {
                (Some(x), Some(y)) => (x, y, format!("{x},{y}")),
                _ => {
                    let els = b.elements()?;
                    let e = ui::find(&els, id.as_deref(), label.as_deref())?;
                    let (x, y) = e.center();
                    (x, y, e.line())
                }
            };
            b.tap(x, y)?;
            std::thread::sleep(Duration::from_millis(500)); // let the UI react before the next command
            println!("tapped {what}");
        }
        Command::Swipe { direction, from, to, ms } => {
            let b = dev()?.backend()?;
            let (from, to) = match (from, to) {
                (Some(f), Some(t)) => (f, t),
                _ => {
                    let (w, h) = b.size()?;
                    let (cx, cy) = (w / 2, h / 2);
                    match direction.as_deref().unwrap_or("up") {
                        "up" => ((cx, h * 3 / 4), (cx, h / 4)),
                        "down" => ((cx, h / 4), (cx, h * 3 / 4)),
                        "left" => ((w * 4 / 5, cy), (w / 5, cy)),
                        "right" => ((w / 5, cy), (w * 4 / 5, cy)),
                        d => anyhow::bail!("unknown direction {d} (up, down, left, right)"),
                    }
                }
            };
            b.swipe(from, to, ms)?;
            std::thread::sleep(Duration::from_millis(400));
            println!("swiped {},{} → {},{}", from.0, from.1, to.0, to.1);
        }
        Command::Type { text, id, label } => {
            let b = dev()?.backend()?;
            if id.is_some() || label.is_some() {
                let els = b.elements()?;
                let (x, y) = ui::find(&els, id.as_deref(), label.as_deref())?.center();
                b.tap(x, y)?;
                std::thread::sleep(Duration::from_millis(500));
            }
            b.type_text(&text)?;
            println!("typed {} characters", text.chars().count());
        }
        Command::Press { button } => {
            dev()?.backend()?.press(&button)?;
            println!("pressed {button}");
        }
        Command::Screenshot { out, max } => {
            let d = dev()?;
            let path = out.unwrap_or_else(|| std::env::temp_dir().join(format!("boop-{}.jpg", std::process::id())));
            d.screenshot(&path, max)?;
            println!("{}", path.display());
        }
        Command::Open { url } => {
            dev()?.open_url(&url)?;
            println!("opened {url}");
        }
        Command::Wda { action } => {
            let d = dev()?;
            let device::Device::IosSim { udid, .. } = &d else {
                anyhow::bail!("WebDriverAgent is for iOS simulators; {} uses adb", d.describe());
            };
            match action {
                WdaAction::Start => println!("{}", wda::ensure(udid)?),
                WdaAction::Stop => println!("{}", if wda::stop(udid) { "stopped" } else { "not running" }),
                WdaAction::Status => match wda::status(udid) {
                    Some(p) => println!("running on :{p} (log: {})", wda::log_path(udid).display()),
                    None => println!("not running"),
                },
            }
        }
        Command::CheckUpdate { stable, nightly, dev } => update::check_update(stable, nightly, dev),
        Command::Update { stable, nightly, dev, force } => update::update(stable, nightly, dev, force),
    }
    Ok(())
}

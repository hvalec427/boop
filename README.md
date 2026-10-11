# boop

Control iOS simulators and Android devices from the command line: read the
element tree (with accessibility ids such as React Native testIDs), tap,
swipe, type, press buttons, take screenshots, open deep links. Made for
coding agents, which call it through their shell, as much as for people and
scripts.

```sh
boop devices                       # booted simulators and adb devices
boop ui                            # Button #login_guest_button "Continue as guest" @201,761
boop ui guest --json               # filter; JSON with frames
boop tap --id login_guest_button   # by id (preferred), --label "Continue", or a point: tap 201 761
boop swipe up                      # finger moves up, content scrolls down; or --from X,Y --to X,Y
boop type "me@example.com" --id email_input
boop press enter                   # home, back (Android), enter
boop screenshot                    # prints the JPEG path (-o file, --max px)
boop open "myapp://prices"
```

Coordinates are what the device taps in: points on iOS, pixels on Android.
`ui` reports centers in the same space, so what you read is what you tap.

## Device

`--device <id>`, else `$BOOP_DEVICE`, else `--device-from <file>`
/ `$BOOP_DEVICE_FROM` (a JSON file with a `"udid"`, e.g. signalbox's
`.signalbox/session.json`, looked up from the current dir upwards), else the
only booted simulator or adb device.

boop, signalbox and simon target a device by the same `--device` id, the one `simon list`,
`simon running`, `boop devices` and signalbox's Devices pane show:

| Device | id |
|---|---|
| iOS simulator or iPhone | its udid |
| Android emulator | its AVD name (its `emulator-5554` serial changes every boot, though that works too) |
| Android phone | its adb serial |

A device name works as well when only one device has it.

## How it works

- **iOS simulators:** WebDriverAgent, the XCTest runner Appium and Maestro use
  underneath. boop clones it into `~/.cache/boop`, builds it once per
  Xcode version (a minute or two), and runs one per simulator in the
  background on its own port. `boop wda status|start|stop` manages it.
  `BOOP_WDA_TAG` picks the release (default `v16.12.10`, the latest that
  builds with Xcode 26.2).
- **Android:** adb (`uiautomator dump`, `input`, `screencap`).
- **Physical iPhones:** not yet (WebDriverAgent needs code signing there).

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/hvalec427/boop/develop/install.sh | sh -s -- dev
```

It installs to `~/.boop/bin/boop` (no sudo) and adds that folder to your PATH in
your shell config.

(A dev build: there is no stable release yet.) Update with
`boop update` (`--stable`, `--nightly`, `--dev` to switch channels).
Uninstall:

```sh
curl -fsSL https://raw.githubusercontent.com/hvalec427/boop/develop/uninstall.sh | sh
```

macOS only (Xcode for iOS, the Android SDK's adb for Android).

boop was called touchctl. The installer removes an old touchctl install, and
the old `TOUCHCTL_*` environment variables still work.

## Contributing

Commits must follow [Conventional Commits](https://www.conventionalcommits.org)
(`feat: …`, `fix(ios): …`) — release versions are derived from them. Enable the
local check once per clone:

```sh
git config core.hooksPath .githooks
```

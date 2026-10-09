# touchctl

Control iOS simulators and Android devices from the command line: read the
element tree (with accessibility ids such as React Native testIDs), tap,
swipe, type, press buttons, take screenshots, open deep links. Made for
coding agents, which call it through their shell, as much as for people and
scripts.

```sh
touchctl devices                       # booted simulators and adb devices
touchctl ui                            # Button #login_guest_button "Continue as guest" @201,761
touchctl ui guest --json               # filter; JSON with frames
touchctl tap --id login_guest_button   # by id (preferred), --label "Continue", or a point: tap 201 761
touchctl swipe up                      # finger moves up, content scrolls down; or --from X,Y --to X,Y
touchctl type "me@example.com" --id email_input
touchctl press enter                   # home, back (Android), enter
touchctl screenshot                    # prints the JPEG path (-o file, --max px)
touchctl open "myapp://prices"
```

Coordinates are what the device taps in: points on iOS, pixels on Android.
`ui` reports centers in the same space, so what you read is what you tap.

## Device

`--device <udid|serial>`, else `$TOUCHCTL_DEVICE`, else `--device-from <file>`
/ `$TOUCHCTL_DEVICE_FROM` (a JSON file with a `"udid"`, e.g. metroctl's
`.metroctl/session.json`, looked up from the current dir upwards), else the
only booted simulator or adb device.

## How it works

- **iOS simulators:** WebDriverAgent, the XCTest runner Appium and Maestro use
  underneath. touchctl clones it into `~/.cache/touchctl`, builds it once per
  Xcode version (a minute or two), and runs one per simulator in the
  background on its own port. `touchctl wda status|start|stop` manages it.
  `TOUCHCTL_WDA_TAG` picks the release (default `v16.12.10`, the latest that
  builds with Xcode 26.2).
- **Android:** adb (`uiautomator dump`, `input`, `screencap`).
- **Physical iPhones:** not yet (WebDriverAgent needs code signing there).

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/hvalec427/touchctl/master/install.sh | sh
```

Or the latest build from `develop` with `… | sh -s -- dev`. Update with
`touchctl update` (`--stable`, `--nightly`, `--dev` to switch channels).
Uninstall:

```sh
curl -fsSL https://raw.githubusercontent.com/hvalec427/touchctl/master/uninstall.sh | sh
```

macOS only (Xcode for iOS, the Android SDK's adb for Android).

## Contributing

Commits must follow [Conventional Commits](https://www.conventionalcommits.org)
(`feat: …`, `fix(ios): …`) — release versions are derived from them. Enable the
local check once per clone:

```sh
git config core.hooksPath .githooks
```

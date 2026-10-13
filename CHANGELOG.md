# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The app version is also the version of the bundled
demo library.

## [Unreleased]

### Changed

- Measurements are named from the app's own config: fragment = the item's German title, series =
  the item id (e.g. `Attika 1.light-5`), so the floorplan picker shows readable names. The
  simulator no longer writes `lw_config*` fragments; applying a demo removes them. DALI groups,
  digital inputs and the time zone live in `lw_demo`. The floorplan plugin's DALI group and Modbus
  write controls stay inactive until the plugin is adapted (DECISIONS D10).
- Control tab: alarms sit in the same list as the items they belong to, the text search also
  finds alarm texts, and an "Active alarms" filter shows only cards with a raised alarm.
- Device overview rows have more space between them.

### Added

- Scenarios have a description and can be edited, created, copied and deleted per device in the
  Setup tab, with value, generator and alarm steps (DECISIONS D11).
- "Reset to defaults" in the Control tab: clears all alarms and sets every value and generator
  back to its default (counters keep their value).
- The left menu can be hidden (button, Cmd/Ctrl+\, left edge); the window works down to 480 px
  wide.

## [0.1.2] - 2026-10-13

### Changed

- Release builds use tauri-action 1.0 and GitHub actions on Node 24. The macOS updater archive
  now carries the version in its file name.

### Fixed

- Windows test executables start again (the app manifest is embedded into every binary); CI is
  green on Windows.

## [0.1.1] - 2026-10-13

### Changed

- The login form asks for the tenant URL and tenant ID; no tenants ship with the app any more.
  "Remember me" stores them with the user name; saved logins from 0.1.0 are not taken over.
- New updater signing key. 0.1.0 cannot update itself to this version: install it by hand.
- macOS builds are signed with the Lichtwart GmbH Developer ID and notarized by Apple.

## [0.1.0] - 2026-10-12

### Added

- First version of the Lichtwart Demo Simulator: sign in to Cumulocity, create and run demo
  devices from the bundled library, raw fire, activity log, app updates.
- Release pipeline: one draft release that is published only after all platform builds succeed,
  version checks for Tauri crates and npm packages, map tiles from `tile.openstreetmap.org`.
- Alarms are fired once from buttons (alarms panel and lamp / sensor menus) and stay in effect
  until they are cleared in Cumulocity; "Clear all" clears every open alarm of the device. The
  session follows clears and acknowledgements made in Cumulocity (realtime `/alarms/<id>`, with a
  re-sync after a reconnect). DECISIONS D9.

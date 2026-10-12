# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). The app version is also the version of the bundled
demo library.

## [Unreleased]

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

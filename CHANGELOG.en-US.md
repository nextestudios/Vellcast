# Changelog

Notes in US English; the Brazilian Portuguese version lives in `CHANGELOG.md`. Before publishing a version, add a `## [VERSION]` section **to both files** (in the `chore(release): VERSION` pull request, together with the `Cargo.toml` version). The release workflow uses the tag's section and fails if either is missing. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]
### New
- **Guest name and participant list in the web player (`/watch`):** whoever opens an invite link enters a name before joining; the invite is only used when the name is submitted (closing the page first does not spend it). After joining, the player shows who is in the room and how many, updating on every join, leave, drop or reconnect, without duplicate names. The publisher appears with the display name from the app settings. Names are always shown as plain text; invites and tokens never appear in the list.
- **Relay:** `POST /api/rooms/{id}/join` accepts `name` (1 to 32 characters, validated before the invite is consumed), the name is signed into the viewer token, and the new `Presence` message (`0x40`) sends the participant list to viewers ([protocol](docs/STREAMING_PROTOCOL.md#33-participantes-presence)). Older players ignore the message.

### Fixes
- **Web player fits the window:** the `/watch` page uses exactly the visible browser area, with no scrollbars. The picture is letterboxed when the stream's aspect ratio differs from the window (never stretched or cropped), follows resizing and fullscreen, and the controls, status bar and statistics stay visible, including on narrow screens.

## [0.2.0-alpha.1]
### New
- **Open source under AGPL-3.0-only:** Vellcast is now free software; the [github.com/nextestudios/Vellcast](https://github.com/nextestudios/Vellcast) repository will be opened once the project is ready. Versions up to 0.1.2 were distributed under a proprietary license ([licensing notes](docs/LICENSING.md)). The commercial services (accounts and plans) stay in a separate private repository and are not needed to stream.
- **Portable version:** `Vellcast-Portable-x64.exe`, a single executable that keeps its settings and logs in the `Vellcast_Data` folder next to it. It tells you about new versions; replacing the file is manual.

### Changes
- **Updates between alpha versions:** the update checker now orders pre-releases by SemVer rules (`0.2.0-alpha.1` < `0.2.0-alpha.2` < `0.2.0`).
- **Releases follow the ControlFS pattern:** published from a tag on `main`, with English and Portuguese notes, the full test suite and the real app launched (portable and installed) before publishing; `SHA256SUMS.txt` with the hash of both downloads.
- The **About** page shows the new license and the source code address.

### Known limitations
- Not yet validated with real GPUs and games or with a physical controller; the Discord integration waits for the Discord application.
- The test server is in Europe: from Brazil, use the **Economy** quality.
- Executables are not code-signed ([policy](docs/CODE_SIGNING.md)): SmartScreen may warn on first run.
- Whoever is on 0.1.1 or 0.1.2 sees this version in **About → Check for updates**.

## [0.1.2]
### Fixes
- **"Could not reach the server" when creating rooms:** the app now validates certificates through Windows itself, like the browser. Antivirus software that inspects HTTPS (Kaspersky, Avast, ESET, Bitdefender) installs its own certificate, which the app did not recognize. Streaming also uses the system certificates now.
- Error messages and the log now show the technical reason for the failure.

## [0.1.1]
### New
First test version of Vellcast: stream your Windows game and watch it in the browser through an invite link.
- Native capture (Windows Graphics Capture) and hardware H.264, with a software fallback.
- Per-process game audio (WASAPI + Opus), without capturing Discord voice.
- Adaptive bitrate and competitive mode.
- Full controller navigation, with an on-screen keyboard.
- Private rooms with temporary invites; test server preconfigured.
- Tray mode, advanced metrics and a log in `%LOCALAPPDATA%\Next Estudios\Vellcast\data\logs`.

[Unreleased]: https://github.com/nextestudios/Vellcast/compare/v0.2.0-alpha.1...HEAD
[0.2.0-alpha.1]: https://github.com/nextestudios/Vellcast/compare/v0.1.2...v0.2.0-alpha.1
[0.1.2]: https://github.com/nextestudios/Vellcast/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/nextestudios/Vellcast/releases/tag/v0.1.1

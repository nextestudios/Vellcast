# Changelog

Notes in US English; the Brazilian Portuguese version lives in `CHANGELOG.md`. Before publishing a version, add a `## [VERSION]` section **to both files** (in the `chore(release): VERSION` pull request, together with the `Cargo.toml` version). The release workflow uses the tag's section and fails if either is missing. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]
### New
- **Guest name and participant list in the web player (`/watch`):** whoever opens an invite link enters a name before joining; the invite is only used when the name is submitted (closing the page first does not spend it). After joining, the player shows who is in the room and how many, updating on every join, leave, drop or reconnect, without duplicate names. The publisher appears with the display name from the app settings. Names are always shown as plain text; invites and tokens never appear in the list.
- **Relay:** `POST /api/rooms/{id}/join` accepts `name` (1 to 32 characters, validated before the invite is consumed), the name is signed into the viewer token, and the new `Presence` message (`0x40`) sends the participant list to viewers ([protocol](docs/STREAMING_PROTOCOL.md#33-participantes-presence)). Older players ignore the message.
- **Rooms from Home:** the Home page now has "Create room" and "Join with a link". Paste an invite link (or select **Paste link**) to join someone else's room; it shows up in Rooms marked as **Participant**.
- **Any participant can share their screen:** in the room, select **Share my screen**, choose what to share and start. One person shares at a time: whoever starts takes the turn, the previous sharer is notified and viewers switch to the new stream without a frozen picture. Stopping, leaving the room or dropping frees the turn immediately.
- The room shows who is sharing (in the app and in the web player) and offers **Watch in browser**, **Stop sharing** and **Leave room**.
- **Quick controls on the Live page:** change what you share, the quality (content, resolution, frame rate and bitrate), the game audio and the microphone without leaving the stream, with a controller, mouse or keyboard. Bitrate, content preference and audio apply instantly; source, resolution and frame rate restart capture and the encoder in the same room, after a confirmation that warns about the brief freeze.
- **Microphone (optional):** pick a microphone and turn it on, off or mute it while streaming. It starts off and is mixed with the game audio, which stays separately controlled (selected app only, entire system without Discord voice, or off).
- **"Sharp text" content preference:** for code and documents, when the network slows down Vellcast lowers the frame rate instead of blurring the text.

### Fixes
- **Web player fits the window:** the `/watch` page uses exactly the visible browser area, with no scrollbars. The picture is letterboxed when the stream's aspect ratio differs from the window (never stretched or cropped), follows resizing and fullscreen, and the controls, status bar and statistics stay visible, including on narrow screens.

### Security
- Only people who joined with a valid invite (or the owner) can ask to share; revoking a participant ends their share. Invites, tokens and names never appear in server logs.

### Changes
- If the shared window or monitor closes, the stream no longer stops right away: viewers see the last image and you have 2 minutes to choose another source.
- Quality options only show what Vellcast and your plan actually offer (up to 1080p60 on the Free plan); unavailable options are listed in a note.

### Known limitations
- A real microphone, switching devices and a physical controller have not been validated on hardware yet; the CI test machine has no microphone.

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

<div align="center" style="border-bottom: none">
    <h1>Oliv — Local Meeting Transcriber</h1>
    <p><b>Privacy-first, on-device meeting transcription for the Oliv platform.</b></p>
    <a href="https://img.shields.io/badge/Supported_OS-macOS,_Windows-white"><img src="https://img.shields.io/badge/Supported_OS-macOS,_Windows-white" alt="Supported OS"></a>
    <a href="https://img.shields.io/badge/License-MIT-blue"><img src="https://img.shields.io/badge/License-MIT-blue" alt="License"></a>
</div>

## What this is

**Oliv** is the desktop transcriber for [Oliv](https://oliv.ai) — a lightweight menubar app that
records and transcribes your meetings **locally**, with no bot joining the call. Audio never leaves
your machine for transcription; the resulting transcript is streamed to your Oliv workspace so it
shows up alongside your other meetings, scorecards, and CRM data.

It runs quietly in the menubar, detects when a meeting app (Zoom, Meet, Teams, Webex, Slack, …)
starts using your microphone, and offers to start transcribing — one click, no setup prompts.

## Features

- **On-device transcription** — speech-to-text runs locally (Parakeet), so meeting audio is never uploaded.
- **Menubar app** — start/stop from the tray; no window required.
- **Automatic meeting detection** — prompts to transcribe when a whitelisted meeting app captures the mic.
- **Sign in with Oliv** — transcripts sync to your Oliv workspace.
- **Sensitive mode** — transcribe only your own voice when a meeting is marked sensitive.
- **Auto-updates** — signed, notarized builds delivered via the in-app updater.

## Install

Download the latest installer for your platform from the **[Releases](../../releases/latest)** page:

- **macOS** — `Oliv.AI_<version>_aarch64.dmg` (Apple Silicon). Open it, drag **Oliv AI** to Applications, launch, then **Sign in with Oliv**.
- **Windows** — `Oliv.AI_<version>_x64-setup.exe`. Run it, launch **Oliv AI**, then **Sign in with Oliv**.

Inside the Oliv web app, the onboarding flow also offers an OS-detected download link.

The first time you start transcription, the app downloads the local speech model
(~640 MB) and shows a brief "Getting you ready…" state — after that it's instant.

## Build from source

Requires Node 20+, pnpm, and the Rust toolchain. macOS builds also require Xcode
(the `cidre` crate links against macOS frameworks), which is why release builds run in CI.

```bash
cd frontend
pnpm install
pnpm tauri build        # production bundle
pnpm tauri dev          # local development
```

CI workflows under `.github/workflows/` produce signed, notarized release artifacts; see
`release.yml` for the publish flow.

## Credits

Oliv is built on [**Meetily**](https://github.com/Zackriya-Solutions/meeting-minutes) by
Zackriya Solutions, an open-source privacy-first meeting assistant, under the MIT License.
We're grateful to the Meetily authors and community.

## License

MIT License — see [LICENSE](LICENSE).

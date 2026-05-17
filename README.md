# Drovix Portal — Desktop

Native desktop client for [portal.drovix.com](https://portal.drovix.com),
built with [Tauri 2](https://tauri.app). Windows, macOS, and Linux.

## What it is

A thin native window around the production web portal. Same UI, same auth,
same data — just wrapped in a real OS window with a taskbar/dock icon,
dedicated process, and (eventually) auto-updates.

Why not just open a browser tab? Institutional desks want a focused window
that lives in the dock, doesn't get lost between 40 browser tabs, and
survives a browser crash. The desktop client is also the future home for
features that need OS integration (native notifications, deep links from
emails, keychain-backed credential storage).

## Architecture

```
┌────────────────────────────────────────────────┐
│  Drovix Portal.exe / .dmg / .AppImage          │
│  ┌──────────────────────────────────────────┐  │
│  │  System WebView                          │  │
│  │  (WebView2 on Win, WKWebView on macOS,   │  │
│  │   WebKitGTK on Linux)                    │  │
│  │           ↓                              │  │
│  │   https://portal.drovix.com (Vercel)     │  │
│  └──────────────────────────────────────────┘  │
└────────────────────────────────────────────────┘
```

- **No bundled web app.** The window points at the live production URL, so
  every Vercel deploy is instantly available to desktop users — no need to
  re-release the installer for a UI bug fix.
- **No Node.js runtime in the binary.** The Rust core is ~3MB, the rest is
  the OS's own webview. Installer is ~10MB vs ~150MB for Electron.
- **Auto-updater hooks** are wired but disabled until the first signed
  release; updates ship as `.tar.gz` / `.zip` deltas via GitHub Releases.

## Local development

Prereqs:

- Rust stable (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Node.js 20+
- Platform deps: see [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/)
  - **Linux/WSL**: `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
  - **macOS**: Xcode Command Line Tools
  - **Windows**: WebView2 Runtime (preinstalled on Win 11), MSVC build tools

```bash
npm install
npm run dev          # opens the window pointing at portal.drovix.com
npm run build        # builds installer for the current host platform
```

The `dist/` folder is just an empty placeholder — Tauri demands a
`frontendDist` to exist on disk even when the window's `url` is remote.

## Icons

Source: `icon-source.png` (1024×1024 RGBA, the Drovix mark).

Regenerate the platform-specific icon set:

```bash
npm run icon         # writes src-tauri/icons/*
```

## Release flow

Releases are produced by GitHub Actions on every `v*.*.*` tag:

1. `git tag v0.1.0 && git push --tags`
2. The workflow builds three matrix jobs in parallel:
   - `windows-latest` → `.msi` + `.exe` (NSIS)
   - `macos-latest` (arm64) → `.dmg` for Apple Silicon
   - `macos-13` (x86_64) → `.dmg` for Intel Macs
   - `ubuntu-latest` → `.AppImage` + `.deb`
3. Artifacts are uploaded to a GitHub Release as a draft.
4. Publish the draft to make the release public.

First release is **unsigned**. Users will see a one-time OS warning:
- **macOS**: right-click the `.dmg` and choose Open the first time.
- **Windows**: SmartScreen "Run anyway".
- **Linux**: no warning.

Code signing setup (Apple Developer Program, Windows EV certificate) is
tracked in `docs/code-signing.md` and will be wired up before institutional
clients onboard.

## Allowlist / security posture

The webview can only talk to:

- `portal.drovix.com` (the main app)
- Any domain it transitively loads (Vercel CDN, Supabase, Sumsub, etc.)

The Tauri↔webview IPC surface exposed to the page is minimal:

- `shell:allow-open` — open external URLs in the user's default browser
- `process:default` — exit / relaunch (used by the updater)
- `updater:default` — check / install updates

No filesystem, no shell command execution, no arbitrary process spawning.

## Related repos

- [Tor2539/portal-drovix.com](https://github.com/Tor2539/portal-drovix.com) — the Next.js portal this client wraps
- [Tor2539/drovix-engine](https://github.com/Tor2539/drovix-engine) — telemetry/control plane
- [Tor2539/Drovix-market-making-engine](https://github.com/Tor2539/Drovix-market-making-engine) — MM engine

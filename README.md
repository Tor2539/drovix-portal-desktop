# Drovix Portal — Desktop

Native desktop client for [portal.drovix.com](https://portal.drovix.com),
built with [Tauri 2](https://tauri.app). Windows, macOS (Apple Silicon) and
Linux.

## Decision: harden (2026-09-21)

Operator decision (takeover plan P11): the desktop wrapper is **kept and
hardened**, not frozen. Reasons, in order:

1. The installer is ~3 MB and carries no web code, so keeping it costs one CI
   run per release and nothing per portal deploy.
2. Institutional desks asked for a dock/taskbar window that survives browser
   crashes; a browser-only notice would remove that without saving effort.
3. The parts that made it "not usable" were mechanical, not architectural:
   no startup update check, `target="_blank"` links dead, an unproven CI
   matrix on deprecated Node-20 actions. All three are fixed in v0.2.0
   (see [Hardening in v0.2.0](#hardening-in-v020) and [Evidence](#evidence)).

Still open, tracked as **procurement, not blocking**:

- Apple Developer Program + Windows EV/OV code-signing certificate. Until
  bought, every build is unsigned and users see the one-time OS warning below.
- A "Download desktop app" link on the portal is added **only after** the
  signing decision, so no client is pointed at an unsigned installer by the
  product itself.

Canonical hostname is `portal.drovix.com`. `portal-drovix.com` (dash) does not
resolve and must not be used anywhere in this repo.

## What it is

A thin native window around the production web portal. Same UI, same auth,
same data — wrapped in a real OS window with a taskbar/dock icon, dedicated
process, an in-app updater and OS-native handling of external links.

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
│  Rust core: window policy, updater, log file   │
└────────────────────────────────────────────────┘
```

- **No bundled web app.** The window points at the live production URL, so
  every Vercel deploy is instantly available to desktop users — no need to
  re-release the installer for a UI bug fix.
- **No Node.js runtime in the binary.** The Rust core is ~3 MB, the rest is
  the OS's own webview.
- **Updater** checks GitHub Releases on every start (Rust side, see below).

## Hardening in v0.2.0

All of it lives in [`src-tauri/src/lib.rs`](src-tauri/src/lib.rs); the main
window is now built there instead of in `tauri.conf.json` so handlers can be
attached.

| Concern | Behaviour | Where |
| --- | --- | --- |
| Updates | On start (+3 s) the Rust side fetches `releases/latest/download/latest.json`, verifies the minisign signature against the pubkey in `tauri.conf.json`, and shows a native "Update now / Later" dialog. Windows: the NSIS/MSI installer is launched and the app exits; macOS/Linux: install then `app.restart()`. Failures are logged, never fatal. | `check_for_updates` |
| `target="_blank"` / `window.open` | Routed by origin: same-origin portal URL navigates the main window in place; any other `https`/`http`/`mailto`/`tel` URL goes to the OS default handler via `tauri-plugin-opener`; `file:`, `javascript:`, `data:` etc. are dropped. The webview **never** creates its own popup (`NewWindowResponse::Deny`). | `classify_new_window`, `on_new_window` |
| Top-level navigation | `https` anywhere (OAuth / KYC / payment providers redirect the top frame), `http` only on loopback, everything else blocked. | `allow_navigation`, `on_navigation` |
| Logging | `tauri-plugin-log` → stdout + platform log dir. Windows: `%LOCALAPPDATA%\com.drovix.portal\logs\drovix-portal.log`; macOS: `~/Library/Logs/com.drovix.portal/`; Linux: `~/.local/share/com.drovix.portal/logs/`. | `run()` |
| Policy tests | `cargo test` covers the two policy functions (portal vs foreign vs dangerous URLs, look-alike hosts such as `portal.drovix.com.evil.example`). | `mod tests` |

### IPC surface exposed to the page: none

Tauri 2 gives a remote origin **no** IPC access unless a capability declares
it under `remote.urls`. [`src-tauri/capabilities/default.json`](src-tauri/capabilities/default.json)
has no `remote` block and is unchanged from v0.1.x, so
`https://portal.drovix.com` cannot call `updater:*`, `process:*`, `opener:*`
or any `core:*` command. The updater, the external-link handling and the
process exit are all driven from Rust. A compromised or spoofed portal page
therefore cannot trigger, suppress or redirect an update, and cannot open
files or processes on the desk's machine.

## Local development

Prereqs:

- Rust stable (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Node.js 22+
- Platform deps: see [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/)
  - **Linux/WSL**: `sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`
  - **macOS**: Xcode Command Line Tools
  - **Windows**: WebView2 Runtime (preinstalled on Win 11), MSVC build tools

```bash
npm ci
npm run dev                       # opens the window pointing at portal.drovix.com
(cd src-tauri && cargo test)      # link / navigation policy tests
npm run build                     # builds installer for the current host platform
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

[`.github/workflows/release.yml`](.github/workflows/release.yml) has two
modes, so a manual run can never produce a stray release:

| Trigger | Builds | Release |
| --- | --- | --- |
| `workflow_dispatch` (any branch) | 3 jobs: `windows-latest`, `macos-latest` (arm64), `ubuntu-22.04` | **none** — bundles land as workflow artifacts (7 days) |
| push of tag `v*.*.*` | same 3 jobs | draft GitHub Release `Drovix Portal vX.Y.Z` with `.msi`, `-setup.exe`, `.dmg`, `.app.tar.gz`, `.AppImage`, `.deb`, `.rpm`, their `.sig` files and `latest.json` |

Cutting a release:

1. Bump the version in `package.json`, `package-lock.json`,
   `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` and `src-tauri/tauri.conf.json`
   (all five must agree — the updater compares `latest.json.version` against
   the Cargo version).
2. Optional smoke run first: `gh workflow run release.yml --ref <branch>` and
   wait for three green jobs; confirm `gh release list` shows nothing new.
3. `git tag vX.Y.Z && git push origin vX.Y.Z` — from `main`, or from the PR
   branch if the PR will be merged with a merge commit (a squash merge leaves
   the tag on an orphaned commit; re-tag after merge in that case).
4. When the three jobs are green: `gh release edit vX.Y.Z --draft=false`.
   Only a published (non-draft, non-prerelease) release is served from
   `releases/latest/download/latest.json`, i.e. only then do installed clients
   see the update on their next start.

First releases are **unsigned**. Users see a one-time OS warning:
- **macOS**: right-click the `.dmg` and choose Open the first time.
- **Windows**: SmartScreen "Run anyway".
- **Linux**: no warning.

Secrets used by the workflow (names only; values live in GitHub repo secrets
and are never committed): `TAURI_SIGNING_PRIVATE_KEY`,
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (updater minisign key, active since
v0.1.0). Code-signing secrets (`APPLE_*`, `WINDOWS_*`) are intentionally not
wired until the certificates are bought.

## Evidence

Recorded 2026-09-21 (lane P11, PR [#1](https://github.com/Tor2539/drovix-portal-desktop/pull/1)).
Times below are UTC as printed by GitHub / the app log.

| # | Claim | Evidence |
| --- | --- | --- |
| 1 | Policy unit tests pass | `cargo test` in `src-tauri`: `test result: ok. 4 passed; 0 failed` (also the `Rust unit tests` step of every CI run below) |
| 2 | `workflow_dispatch` smoke run, 3 green jobs, no release created | run [35529502572](https://github.com/Tor2539/drovix-portal-desktop/actions/runs/35529502572) on `63101b2`: windows-latest / macos-latest / ubuntu-22.04 all `success`; artifacts `drovix-portal-x86_64-pc-windows-msvc` (4.27 MB), `-aarch64-apple-darwin` (5.98 MB), `-x86_64-unknown-linux-gnu` (97.3 MB); `gh release list` afterwards still showed only v0.1.1 / v0.1.0 (no `main` release). One annotation (upload-artifact@v4 on Node 20) fixed by `f9d214c`. |
| 3 | Second smoke run on the final workflow, zero annotations | run [35530174083](https://github.com/Tor2539/drovix-portal-desktop/actions/runs/35530174083) on `f9d214c`: 3 × `success`, no annotations, no release |
| 4 | Tag build produces the release with `latest.json` | tag `v0.2.0` → `f9d214c`; run [35530178290](https://github.com/Tor2539/drovix-portal-desktop/actions/runs/35530178290): 3 × `success`, no annotations; draft release created with 14 assets (`.msi`, `-setup.exe`, `.dmg`, `.app.tar.gz`, `.AppImage`, `.deb`, `.rpm`, 7 × `.sig`, `latest.json` 5785 B) |
| 5 | Release published, updater endpoint serves it | `gh release edit v0.2.0 --draft=false` at 18:59:40Z → [releases/tag/v0.2.0](https://github.com/Tor2539/drovix-portal-desktop/releases/tag/v0.2.0). `curl -sL .../releases/latest/download/latest.json` → `version: 0.2.0`, `windows-x86_64` → `Drovix.Portal_0.2.0_x64-setup.exe` (NSIS preferred), plus msi / darwin-aarch64 / linux appimage+deb+rpm entries, every one with a minisign signature |
| 6 | Install on the Windows workstation reaches `/login` | Local build of the same source (`npx tauri build --bundles nsis,msi`, updater artifacts off because the signing key is CI-only) → `Drovix Portal_0.2.0_x64-setup.exe` (sha256 `2a6ec30b…9501`) installed with `/S`; registry `HKCU\...\Uninstall` shows `Drovix Portal 0.2.0`, `C:\Users\<user>\AppData\Local\Drovix Portal\drovix-portal-desktop.exe` ProductVersion 0.2.0. Launched: PowerShell `MainWindowTitle = "Drovix Portal"`; screenshot [docs/evidence/2026-09-21-v0.2.0-windows-login.png](docs/evidence/2026-09-21-v0.2.0-windows-login.png) shows the login form; app log [docs/evidence/2026-09-21-v0.2.0-windows-startup.log](docs/evidence/2026-09-21-v0.2.0-windows-startup.log): `navigation: https://portal.drovix.com/login?redirect=%2Fportal%2Fdashboard` then `updater: no update available` (v0.1.1 was latest at that moment) |
| 7 | Rust-side updater finds and offers a real release | A throw-away binary built with `--config '{"version":"0.1.9"}'` (not installed, not published) started after v0.2.0 was public: log [docs/evidence/2026-09-21-v0.1.9-test-binary-updater.log](docs/evidence/2026-09-21-v0.1.9-test-binary-updater.log) `updater: 0.1.9 -> 0.2.0 available (…/v0.2.0/Drovix.Portal_0.2.0_x64-setup.exe)` and the native dialog [docs/evidence/2026-09-21-v0.1.9-test-binary-update-dialog.png](docs/evidence/2026-09-21-v0.1.9-test-binary-update-dialog.png) ("Update now" / "Later"). The install path itself was not exercised on the workstation (process killed at the dialog). |
| 8 | Remote origin has no IPC | `src-tauri/capabilities/default.json` has no `remote` key (Tauri 2 default: remote URLs get no IPC); the only edit is the local-window permission rename `shell:allow-open` → `opener:allow-open-url` |

Not verified at runtime (unit-tested only): the `on_new_window` routing of a
real `target="_blank"` click inside the installed app. The handler is wired
in `build_main_window` and the policy has tests; a manual click test on the
portal's Terms/Privacy links is the remaining check.

Secrets used: none committed. The workflow reads `TAURI_SIGNING_PRIVATE_KEY`
and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from repo secrets; the local build
ran without them.

## Related repos

- [Tor2539/portal-drovix.com](https://github.com/Tor2539/portal-drovix.com) — the Next.js portal this client wraps
- [Tor2539/drovix-engine](https://github.com/Tor2539/drovix-engine) — telemetry/control plane
- [Tor2539/Drovix-market-making-engine](https://github.com/Tor2539/Drovix-market-making-engine) — MM engine

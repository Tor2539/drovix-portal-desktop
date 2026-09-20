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
   (see [Hardening in v0.2.x](#hardening-in-v02x) and [Evidence](#evidence)).
   v0.2.1 fixes what the review of v0.2.0 found: the release exe was a
   console-subsystem binary (a console window opened next to the portal),
   plain `mailto:` / `tel:` links were blocked, and the log kept full URLs.

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

## Hardening in v0.2.x

All of it lives in [`src-tauri/src/lib.rs`](src-tauri/src/lib.rs); the main
window is built there instead of in `tauri.conf.json` so handlers can be
attached. The one line that is not in `lib.rs` is the
`windows_subsystem = "windows"` attribute in
[`src-tauri/src/main.rs`](src-tauri/src/main.rs): Rust honours it only on the
binary crate, so v0.2.0 (which had it in `lib.rs`) shipped as a
console-subsystem exe and opened a console window next to the portal window.
v0.2.1 moves it; the PE `Subsystem` field is checked in the evidence table.

| Concern | Behaviour | Where |
| --- | --- | --- |
| Update check | On start (+3 s) the Rust side fetches `releases/latest/download/latest.json` and, if it lists a newer version, shows a native "Update now / Later" dialog. The check itself does **not** verify anything; failures are logged (`updater: check failed`), never fatal. | `check_for_updates`, `updater.check()` |
| Update install | After "Update now", `download_and_install` downloads the bundle for `{os}-{arch}-{installer}` (falling back to `{os}-{arch}`) and verifies its minisign signature against `plugins.updater.pubkey` in `tauri.conf.json`; a bad or mismatched signature aborts before anything is written. Windows: the NSIS (or MSI, for MSI-installed clients) installer is launched and the app exits (exercised end-to-end on Windows on 2026-09-21, evidence row 11). macOS/Linux: install then `app.restart()` — **not verified at runtime**, no macOS/Linux desk is available. | `download_and_install` |
| `target="_blank"` / `window.open` | Routed by origin: same-origin portal URL (scheme + host + port) navigates the main window in place; any other `https`/`http`/`mailto`/`tel` URL goes to the OS default handler via `tauri-plugin-opener`; `file:`, `javascript:`, `data:` etc. are dropped. The webview **never** creates its own popup (`NewWindowResponse::Deny`). | `classify_new_window`, `on_new_window` |
| Top-level navigation | `https` anywhere (OAuth / KYC / payment providers redirect the top frame), `http` only on loopback, `about:` (webview internal). `mailto:` / `tel:` reached by a plain anchor or `location.href = "mailto:..."` (the portal's compliance / support contacts) are handed to the OS handler and the webview stays put — v0.2.0 blocked them. Everything else is refused. | `classify_navigation`, `on_navigation` |
| Logging | `tauri-plugin-log` → stdout (only visible under `tauri dev`) + platform log dir. Windows: `%LOCALAPPDATA%\com.drovix.portal\logs\drovix-portal.log`; macOS: `~/Library/Logs/com.drovix.portal/`; Linux: `~/.local/share/com.drovix.portal/logs/`. URLs are logged as origin + path only (no query string / fragment, `mailto:`/`tel:` reduced to the scheme) because the portal carries one-time secrets in `/set-password?code=` and `/apply/verify?token=`. | `run()`, `loggable` |
| Policy tests | `cargo test` (6 tests) covers the three policy functions and the log redaction: portal vs foreign vs dangerous URLs, look-alike hosts such as `portal.drovix.com.evil.example`, port mismatch (`portal.drovix.com:8443` opens externally), `tel:`, top-level `mailto:`/`tel:`, query/fragment stripping. | `mod tests` |

### IPC surface exposed to the page: none

Tauri 2 gives a remote origin **no** IPC access unless a capability declares
it under `remote.urls`. [`src-tauri/capabilities/default.json`](src-tauri/capabilities/default.json)
has no `remote` block, so `https://portal.drovix.com` cannot call any
command. Since v0.2.1 the capability grants only `core:default` to the local
`main` window: the `opener:allow-open-url`, `process:default` and
`updater:default` grants of v0.1.x/v0.2.0 were dead weight (the updater, the
dialog, the external-link handling and the restart are all called from Rust,
which needs no command permission) and are gone. A compromised or spoofed
portal page therefore cannot trigger, suppress or redirect an update, and
cannot open files or processes on the desk's machine.

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
(cd src-tauri && cargo test)      # link / navigation policy + log redaction tests
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

`latest.json` carries both Windows installers: `windows-x86_64` /
`windows-x86_64-nsis` → `-setup.exe` (`updaterJsonPreferNsis: true`) and
`windows-x86_64-msi` → `.msi`. The updater plugin looks up
`{os}-{arch}-{installer}` first (the bundler stamps the installer type into
the binary), so an MSI-installed client updates with the MSI and an NSIS
client with the NSIS installer; the two never mix.

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

Pending: **v0.2.1 is not tagged or released yet.** The published v0.2.0
(`f9d214c`) still has the console-window and `mailto:` defects listed under
"Decision: harden". After PR #1 is merged (merge commit), tag the merge commit
`v0.2.1`, wait for the three green jobs and publish the draft; installed
0.2.0 clients then get the fix through the in-app updater.

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
| 8 | Remote origin has no IPC | `src-tauri/capabilities/default.json` has no `remote` key (Tauri 2 default: remote URLs get no IPC). v0.2.0 renamed the local-window permission `shell:allow-open` → `opener:allow-open-url`; v0.2.1 drops it together with `process:default` and `updater:default` (only `core:default` remains) — see row 12 for the runtime check. |

Rows 9-12 were recorded on 2026-09-21 for v0.2.1 (review fix pass of PR #1; the CI smoke run on the final commit is linked from the PR body).

| # | Claim | Evidence |
| --- | --- | --- |
| 9 | Release exe is a GUI-subsystem binary (no console window) | `npx tauri build --bundles nsis` of v0.2.1 on the Windows workstation → `src-tauri/target/release/drovix-portal-desktop.exe`: PE optional-header `Subsystem` = **2** (`IMAGE_SUBSYSTEM_WINDOWS_GUI`). The v0.2.0 build of the same tree read **3** (`IMAGE_SUBSYSTEM_WINDOWS_CUI`) before the attribute was moved to `main.rs`. Installer `Drovix Portal_0.2.1_x64-setup.exe` (1 894 367 B) sha256 `cbadbb500833332674482ae99c4265afc8269b21e44ae2133270fb1443053ddc`; the exe it installs also reads `Subsystem` = 2 and, once launched, has no `conhost.exe` child (only `msedgewebview2.exe`). |
| 10 | Policy + redaction unit tests | `cargo test --locked` in `src-tauri`: `test result: ok. 6 passed; 0 failed` (`portal_links_navigate_main_window`, `foreign_links_open_externally` incl. `tel:` and `:8443`, `dangerous_schemes_are_denied`, `top_level_navigation_policy`, `top_level_mailto_and_tel_go_to_the_os`, `log_lines_drop_query_fragment_and_contact_details`); `cargo fmt --check` and `cargo clippy --locked` clean. |
| 11 | Signed download + install path works end-to-end on Windows | A throw-away binary of the v0.2.1 source built with `--config '{"version":"0.1.9"}' --no-bundle` was started on the workstation and "Update now" was pressed: [docs/evidence/2026-09-21-v0.2.1-update-e2e.log](docs/evidence/2026-09-21-v0.2.1-update-e2e.log) shows `updater: 0.1.9 -> 0.2.0 available (…/v0.2.0/Drovix.Portal_0.2.0_x64-setup.exe)`, `updater: downloaded 1893652 bytes`, `updater: download finished, installing`, then the process exited, the NSIS installer ran (`/P /R`) and the freshly installed app relaunched itself: `Drovix Portal desktop 0.2.0 starting` … `updater: no update available`. `download_and_install` only reaches the install step after the minisign signature has verified against the embedded pubkey, so this also proves the CI signing key matches `plugins.updater.pubkey` (cross-check: minisign key id `073888bc5fe851f9` is identical in the pubkey and in the published `Drovix.Portal_0.2.0_x64-setup.exe.sig`). Registry afterwards: `Drovix Portal 0.2.0`, exe `ProductVersion 0.2.0`. |
| 12 | v0.2.1 build runs with the reduced capability and redacted log | The local `Drovix Portal_0.2.1_x64-setup.exe` installed with `/S` over the 0.2.0 from row 11: registry `Drovix Portal 0.2.1`, exe `ProductVersion 0.2.1` sha256 `7e9138d233eea2087db826dce543cfcd0f581050c67fa36ca8dc077aa3457a0d`; launched, window title `Drovix Portal`; [docs/evidence/2026-09-21-v0.2.1-windows-startup.log](docs/evidence/2026-09-21-v0.2.1-windows-startup.log): `navigation: https://portal.drovix.com/login` (the `?redirect=%2Fportal%2Fdashboard` query that v0.2.0 logged is gone), `updater: checking (current 0.2.1)`, `updater: no update available`. |

Not verified at runtime (unit-tested only): the `on_new_window` routing of a
real `target="_blank"` click inside the installed app, and a real
`mailto:` click (the login page has no such link; the policy is covered by
`top_level_mailto_and_tel_go_to_the_os`). The handlers are wired in
`build_main_window`; a manual click test on the portal's Terms/Privacy links
and on the onboarding "Contact compliance" button is the remaining check.
macOS/Linux install + `app.restart()` is not verified either (no desk).

Secrets used: none committed. The workflow reads `TAURI_SIGNING_PRIVATE_KEY`
and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` from repo secrets; the local build
ran without them.

## Related repos

- [Tor2539/portal-drovix.com](https://github.com/Tor2539/portal-drovix.com) — the Next.js portal this client wraps
- [Tor2539/drovix-engine](https://github.com/Tor2539/drovix-engine) — telemetry/control plane
- [Tor2539/Drovix-market-making-engine](https://github.com/Tor2539/Drovix-market-making-engine) — MM engine

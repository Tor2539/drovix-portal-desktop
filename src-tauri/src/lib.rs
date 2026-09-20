// Drovix Portal desktop — Tauri entry point.
//
// This is a thin wrapper around https://portal.drovix.com. The webview is the
// system webview (WebView2 / WKWebView / WebKitGTK) so the binary stays small
// and browser security updates flow through the OS. The application UI is the
// production Next.js app deployed at portal.drovix.com — desktop never forks
// from web.
//
// Hardening decisions (see README "Decision: harden"):
//
// * The main window is built here in Rust (not from `tauri.conf.json`) so we
//   can attach `on_new_window` / `on_navigation` handlers. Everything that the
//   page tries to open in a new window (`target="_blank"`, `window.open`) is
//   routed: same-origin portal links navigate the main window in place,
//   anything else goes to the user's default browser. The webview never
//   spawns a second browser-style popup. Top-level `mailto:` / `tel:`
//   navigations (plain `<a href="mailto:...">`, `location.href = "mailto:..."`)
//   are handed to the OS handler and the webview stays where it is.
//
// * The updater runs on the Rust side, once, shortly after startup. The
//   remote page has NO IPC access: `capabilities/default.json` has no
//   `remote.urls` block and grants only `core:default`, so no `updater:*`,
//   `process:*`, `dialog:*` or `opener:*` command exists for
//   portal.drovix.com to call. The page cannot trigger, spoof or suppress an
//   update; only this binary can.
//
// * Logging goes to the platform log dir (Windows:
//   %LOCALAPPDATA%\com.drovix.portal\logs\drovix-portal.log) so updater and
//   navigation decisions are traceable after the fact. URLs are written
//   without query string / fragment (see `loggable`) because the portal
//   carries one-time secrets there. The stdout target only matters for
//   `tauri dev`: release builds are GUI-subsystem binaries (the
//   `windows_subsystem` attribute lives in `main.rs`, where Rust honours it).

use tauri::{
    webview::NewWindowResponse, AppHandle, Manager, Runtime, Theme, Url, WebviewUrl,
    WebviewWindowBuilder,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::UpdaterExt;

/// Canonical portal host. `portal-drovix.com` (with a dash) does not resolve;
/// the production hostname is `portal.drovix.com`.
pub const PORTAL_ORIGIN: &str = "https://portal.drovix.com";
const MAIN_WINDOW_LABEL: &str = "main";

/// Delay before the first update check so the portal has a chance to paint
/// before any dialog can appear.
const UPDATE_CHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(3);

/// True when `url` is served by the portal itself (same scheme + host + port).
pub fn is_portal_url(url: &Url) -> bool {
    let portal = Url::parse(PORTAL_ORIGIN).expect("PORTAL_ORIGIN is a valid URL");
    url.scheme() == portal.scheme()
        && url.host_str() == portal.host_str()
        && url.port_or_known_default() == portal.port_or_known_default()
}

/// What to do with a URL the page asked to open in a new window/tab.
#[derive(Debug, PartialEq, Eq)]
pub enum NewWindowPolicy {
    /// Same-origin portal page: navigate the main window in place.
    NavigateMain,
    /// Any other http(s) URL (docs, receipts, Sumsub, mailto...): hand it to
    /// the OS default browser / handler.
    OpenExternally,
    /// Schemes we never forward anywhere (file:, javascript:, data:, ...).
    Deny,
}

pub fn classify_new_window(url: &Url) -> NewWindowPolicy {
    match url.scheme() {
        "https" if is_portal_url(url) => NewWindowPolicy::NavigateMain,
        "https" | "http" | "mailto" | "tel" => NewWindowPolicy::OpenExternally,
        _ => NewWindowPolicy::Deny,
    }
}

/// What to do with a top-level navigation of the main webview.
#[derive(Debug, PartialEq, Eq)]
pub enum NavigationPolicy {
    /// Let the webview navigate: `https` anywhere (OAuth / KYC / payment
    /// providers redirect the top frame across origins), `http` on the
    /// dev-only loopback, and `about:` (WebView2 / WebKit use `about:blank`
    /// internally).
    Allow,
    /// `mailto:` / `tel:` reached by a plain anchor or `location.href`
    /// assignment: hand the URL to the OS handler and keep the webview where
    /// it is. The portal uses these for its compliance / support contacts.
    OpenExternally,
    /// Refuse so the webview can never be steered to `file:`, `data:`,
    /// `javascript:` or a custom scheme.
    Deny,
}

pub fn classify_navigation(url: &Url) -> NavigationPolicy {
    match url.scheme() {
        "https" | "about" => NavigationPolicy::Allow,
        "http" if matches!(url.host_str(), Some("localhost") | Some("127.0.0.1")) => {
            NavigationPolicy::Allow
        }
        "mailto" | "tel" => NavigationPolicy::OpenExternally,
        _ => NavigationPolicy::Deny,
    }
}

/// URL as it is written to the log file: origin (scheme, host, non-default
/// port) plus path for http(s), the bare scheme for everything else. Query
/// strings and fragments are dropped because the portal carries one-time
/// secrets in them (`/set-password?code=`, `/apply/verify?token=`) and the
/// log is a plaintext file in the user profile; `mailto:` / `tel:` paths are
/// contact details and are dropped for the same reason.
pub fn loggable(url: &Url) -> String {
    match url.scheme() {
        "http" | "https" => format!("{}{}", url.origin().ascii_serialization(), url.path()),
        "about" => url.as_str().to_owned(),
        scheme => format!("{scheme}:<redacted>"),
    }
}

fn open_externally<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    match app.opener().open_url(url.as_str(), None::<&str>) {
        Ok(()) => log::info!("opened externally: {}", loggable(url)),
        Err(e) => log::warn!("failed to open {} externally: {e}", loggable(url)),
    }
}

fn build_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let portal = Url::parse(PORTAL_ORIGIN).expect("PORTAL_ORIGIN is a valid URL");
    let handle_for_navigation = app.clone();
    let handle_for_new_window = app.clone();

    WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::External(portal))
        .title("Drovix Portal")
        .inner_size(1440.0, 900.0)
        .min_inner_size(1024.0, 640.0)
        .center()
        .resizable(true)
        .decorations(true)
        .theme(Some(Theme::Dark))
        .on_navigation(move |url| {
            let app = &handle_for_navigation;
            match classify_navigation(url) {
                NavigationPolicy::Allow => {
                    log::info!("navigation: {}", loggable(url));
                    true
                }
                NavigationPolicy::OpenExternally => {
                    log::info!("top-level {} handed to the OS handler", loggable(url));
                    open_externally(app, url);
                    false
                }
                NavigationPolicy::Deny => {
                    log::warn!("blocked top-level navigation to {}", loggable(url));
                    false
                }
            }
        })
        .on_new_window(move |url, _features| {
            let app = &handle_for_new_window;
            match classify_new_window(&url) {
                NewWindowPolicy::NavigateMain => {
                    log::info!(
                        "new-window request for portal URL, navigating main: {}",
                        loggable(&url)
                    );
                    match app.get_webview_window(MAIN_WINDOW_LABEL) {
                        Some(main) => {
                            if let Err(e) = main.navigate(url.clone()) {
                                log::warn!("navigate main to {} failed: {e}", loggable(&url));
                            }
                        }
                        None => log::warn!("main window missing; dropping {}", loggable(&url)),
                    }
                }
                NewWindowPolicy::OpenExternally => open_externally(app, &url),
                NewWindowPolicy::Deny => log::warn!("denied new window for {}", loggable(&url)),
            }
            // Never let the webview spawn its own popup window.
            NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

/// Rust-side updater: check once at startup, ask the user, install, restart.
/// `check()` only fetches and parses `latest.json`; the minisign signature of
/// the downloaded bundle is verified against `plugins.updater.pubkey` inside
/// `download_and_install`, so a key mismatch surfaces as `updater: check
/// failed` after the user clicks "Update now", never as a silent install.
/// Errors are logged, never surfaced as a crash - a broken updater endpoint
/// must not stop the desk from reaching the portal.
async fn check_for_updates<R: Runtime>(app: AppHandle<R>) -> tauri_plugin_updater::Result<()> {
    let updater = app.updater_builder().build()?;
    log::info!("updater: checking (current {})", app.package_info().version);
    let Some(update) = updater.check().await? else {
        log::info!("updater: no update available");
        return Ok(());
    };
    log::info!(
        "updater: {} -> {} available ({})",
        update.current_version,
        update.version,
        update.download_url
    );

    let accepted = app
        .dialog()
        .message(format!(
            "Drovix Portal {} is available (you are running {}).\n\nInstall it now? The portal will reopen after the update.",
            update.version, update.current_version
        ))
        .title("Drovix Portal update")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Update now".into(),
            "Later".into(),
        ))
        .blocking_show();
    if !accepted {
        log::info!("updater: user deferred {}", update.version);
        return Ok(());
    }

    let mut downloaded: usize = 0;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk;
                if let Some(total) = total {
                    if downloaded as u64 == total {
                        log::info!("updater: downloaded {total} bytes");
                    }
                }
            },
            || log::info!("updater: download finished, installing"),
        )
        .await?;
    // On Windows the plugin launches the installer and exits the process
    // itself; on macOS/Linux we get here and relaunch the new binary.
    log::info!("updater: installed, restarting");
    app.restart();
}

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("drovix-portal".into()),
                    }),
                ])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            log::info!(
                "Drovix Portal desktop {} starting, url={PORTAL_ORIGIN}",
                app.package_info().version
            );
            build_main_window(app.handle())?;

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(UPDATE_CHECK_DELAY).await;
                if let Err(e) = check_for_updates(handle).await {
                    log::warn!("updater: check failed: {e}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn portal_links_navigate_main_window() {
        assert_eq!(
            classify_new_window(&u("https://portal.drovix.com/portal/statements?x=1")),
            NewWindowPolicy::NavigateMain
        );
        assert_eq!(
            classify_new_window(&u("https://portal.drovix.com:443/login")),
            NewWindowPolicy::NavigateMain
        );
    }

    #[test]
    fn foreign_links_open_externally() {
        for s in [
            "https://docs.drovix.com/x",
            "https://portal.drovix.com.evil.example/",
            "https://evil.example/?u=https://portal.drovix.com",
            "https://portal.drovix.com:8443/portal/dashboard",
            "http://portal.drovix.com/",
            "mailto:support@drovix.com",
            "tel:+41445551234",
        ] {
            assert_eq!(
                classify_new_window(&u(s)),
                NewWindowPolicy::OpenExternally,
                "{s}"
            );
        }
    }

    #[test]
    fn dangerous_schemes_are_denied() {
        for s in [
            "file:///C:/Windows/system.ini",
            "javascript:alert(1)",
            "data:text/html,hi",
        ] {
            assert_eq!(classify_new_window(&u(s)), NewWindowPolicy::Deny, "{s}");
        }
    }

    #[test]
    fn top_level_navigation_policy() {
        for s in [
            "https://portal.drovix.com/login",
            "https://accounts.google.com/o/oauth2/auth",
            "http://localhost:3000/login",
            "http://127.0.0.1:3000/login",
            "about:blank",
        ] {
            assert_eq!(classify_navigation(&u(s)), NavigationPolicy::Allow, "{s}");
        }
        for s in [
            "http://portal.drovix.com/login",
            "file:///etc/passwd",
            "javascript:void(0)",
            "data:text/html,hi",
            "drovix://x",
        ] {
            assert_eq!(classify_navigation(&u(s)), NavigationPolicy::Deny, "{s}");
        }
    }

    #[test]
    fn top_level_mailto_and_tel_go_to_the_os() {
        // completion-gate.tsx / kyb-verification-widget.tsx use plain anchors,
        // onboarding/page.tsx assigns `location.href = "mailto:..."`: all of
        // them arrive here as top-level navigations, not new-window requests.
        for s in ["mailto:compliance@drovix.com", "tel:+41445551234"] {
            assert_eq!(
                classify_navigation(&u(s)),
                NavigationPolicy::OpenExternally,
                "{s}"
            );
        }
    }

    #[test]
    fn log_lines_drop_query_fragment_and_contact_details() {
        assert_eq!(
            loggable(&u("https://portal.drovix.com/set-password?code=SECRET#x")),
            "https://portal.drovix.com/set-password"
        );
        assert_eq!(
            loggable(&u("https://portal.drovix.com:443/login?redirect=%2Fportal")),
            "https://portal.drovix.com/login"
        );
        assert_eq!(
            loggable(&u("http://localhost:3000/apply/verify?token=SECRET")),
            "http://localhost:3000/apply/verify"
        );
        assert_eq!(
            loggable(&u("mailto:compliance@drovix.com")),
            "mailto:<redacted>"
        );
        assert_eq!(loggable(&u("tel:+41445551234")), "tel:<redacted>");
        assert_eq!(loggable(&u("about:blank")), "about:blank");
    }
}

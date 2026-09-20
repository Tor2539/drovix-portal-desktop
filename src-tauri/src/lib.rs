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
//   spawns a second browser-style popup.
//
// * The updater runs on the Rust side, once, shortly after startup. The
//   remote page has NO IPC access: `capabilities/default.json` has no
//   `remote.urls` block, so `updater:*`, `process:*` and `opener:*` commands
//   are unreachable from portal.drovix.com. The page cannot trigger, spoof or
//   suppress an update; only this binary can.
//
// * Logging goes to the platform log dir (Windows:
//   %LOCALAPPDATA%\com.drovix.portal\logs\drovix-portal.log) so updater and
//   navigation decisions are traceable after the fact. The binary is built
//   with `windows_subsystem = "windows"`, so stdout is not visible anyway.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

/// Top-level navigation policy for the main webview. OAuth / KYC / payment
/// providers redirect the top frame across origins, so cross-origin https is
/// allowed; anything that is not https (or the dev-only http loopback) is
/// refused so the webview can never be steered to file:, data: or a custom
/// scheme.
pub fn allow_navigation(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => matches!(url.host_str(), Some("localhost") | Some("127.0.0.1")),
        "about" => true,
        _ => false,
    }
}

fn open_externally<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    match app.opener().open_url(url.as_str(), None::<&str>) {
        Ok(()) => log::info!("opened externally: {url}"),
        Err(e) => log::warn!("failed to open {url} externally: {e}"),
    }
}

fn build_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let portal = Url::parse(PORTAL_ORIGIN).expect("PORTAL_ORIGIN is a valid URL");
    let handle_for_new_window = app.clone();

    WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::External(portal))
        .title("Drovix Portal")
        .inner_size(1440.0, 900.0)
        .min_inner_size(1024.0, 640.0)
        .center()
        .resizable(true)
        .decorations(true)
        .theme(Some(Theme::Dark))
        .on_navigation(|url| {
            let allowed = allow_navigation(url);
            if !allowed {
                log::warn!("blocked top-level navigation to {url}");
            }
            allowed
        })
        .on_new_window(move |url, _features| {
            let app = &handle_for_new_window;
            match classify_new_window(&url) {
                NewWindowPolicy::NavigateMain => {
                    log::info!("new-window request for portal URL, navigating main: {url}");
                    match app.get_webview_window(MAIN_WINDOW_LABEL) {
                        Some(main) => {
                            if let Err(e) = main.navigate(url.clone()) {
                                log::warn!("navigate main to {url} failed: {e}");
                            }
                        }
                        None => log::warn!("main window missing; dropping {url}"),
                    }
                }
                NewWindowPolicy::OpenExternally => open_externally(app, &url),
                NewWindowPolicy::Deny => log::warn!("denied new window for {url}"),
            }
            // Never let the webview spawn its own popup window.
            NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

/// Rust-side updater: check once at startup, ask the user, install, restart.
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
            "http://portal.drovix.com/",
            "mailto:support@drovix.com",
        ] {
            assert_eq!(classify_new_window(&u(s)), NewWindowPolicy::OpenExternally, "{s}");
        }
    }

    #[test]
    fn dangerous_schemes_are_denied() {
        for s in ["file:///C:/Windows/system.ini", "javascript:alert(1)", "data:text/html,hi"] {
            assert_eq!(classify_new_window(&u(s)), NewWindowPolicy::Deny, "{s}");
        }
    }

    #[test]
    fn top_level_navigation_policy() {
        assert!(allow_navigation(&u("https://portal.drovix.com/login")));
        assert!(allow_navigation(&u("https://accounts.google.com/o/oauth2/auth")));
        assert!(allow_navigation(&u("http://localhost:3000/login")));
        assert!(!allow_navigation(&u("http://portal.drovix.com/login")));
        assert!(!allow_navigation(&u("file:///etc/passwd")));
        assert!(!allow_navigation(&u("javascript:void(0)")));
    }
}

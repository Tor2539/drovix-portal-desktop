// Drovix Portal desktop — Tauri entry point.
//
// This is a thin wrapper around portal.drovix.com. The webview is the system
// webview (WebView2 / WKWebView / WebKitGTK) so the binary stays small (~10MB)
// and security updates flow through the OS. The actual application UI is the
// production Next.js app deployed at portal.drovix.com — desktop never forks
// from web.
//
// Allowlist is intentionally tight: shell.open is the only privileged path we
// expose, used to open external links (e.g. Sumsub docs, Stripe receipts) in
// the user's default browser instead of inside the portal webview.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

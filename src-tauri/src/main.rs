// Release builds must be GUI-subsystem binaries, otherwise every launch opens
// a console window next to the portal window (and closing that console kills
// the app). Rust honours `windows_subsystem` only on the binary crate, so the
// attribute has to live here, not in lib.rs (it was ignored there in v0.2.0;
// the PE Subsystem field of that build is 3 = CUI).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    drovix_portal_desktop_lib::run()
}

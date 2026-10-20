// Previene la consola extra en Windows en builds de release; irrelevante en dev.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    vibe_iptv_lib::run();
}

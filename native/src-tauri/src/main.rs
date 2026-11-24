// no console window beside the app on Windows release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    caley_native_lib::run()
}

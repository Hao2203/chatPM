// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = chatpm_lib::run() {
        eprintln!("应用启动失败: {e}");
        std::process::exit(1);
    }
}

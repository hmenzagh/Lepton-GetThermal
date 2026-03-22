#[allow(unused)]
mod commands;
#[allow(unused)]
mod processing;
#[allow(unused)]
mod camera;
#[allow(unused)]
mod uvc_ffi;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

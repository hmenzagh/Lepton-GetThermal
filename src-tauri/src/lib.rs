mod avfoundation;
mod camera;
mod commands;
mod processing;
mod usb_control;

use camera::acquisition::CameraAcquisition;
use camera::lepton::LeptonController;
use parking_lot::Mutex;
use std::sync::Arc;

pub struct AppState {
    pub camera: Mutex<Option<CameraAcquisition>>,
    pub lepton: Mutex<Option<Arc<LeptonController>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            camera: Mutex::new(None),
            lepton: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::stream::connect_camera,
            commands::stream::start_stream,
            commands::stream::stop_stream,
            commands::stream::set_palette,
            commands::controls::perform_ffc,
            commands::controls::get_agc_enable,
            commands::controls::set_agc_enable,
            commands::controls::get_agc_policy,
            commands::controls::set_agc_policy,
            commands::controls::get_polarity,
            commands::controls::set_polarity,
            commands::controls::get_gain_mode,
            commands::controls::set_gain_mode,
            commands::controls::get_device_info,
            commands::controls::get_spotmeter_roi,
            commands::controls::set_spotmeter_roi,
            commands::controls::get_spot_temperature,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

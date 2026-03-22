use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::camera::acquisition::CameraAcquisition;
use crate::processing::palettes::Palette;
use crate::AppState;

#[derive(Clone, Serialize)]
struct FrameEvent {
    /// Base64-encoded RGBA pixel data
    data: String,
    width: usize,
    height: usize,
    min_val: u16,
    max_val: u16,
}

#[tauri::command]
pub fn connect_camera(state: State<'_, AppState>) -> Result<String, String> {
    eprintln!("[thermal-v2] Attempting to connect to PureThermal device...");
    let cam = CameraAcquisition::connect().map_err(|e| {
        eprintln!("[thermal-v2] Connection failed: {e}");
        e.to_string()
    })?;
    eprintln!("[thermal-v2] Device opened successfully");

    *state.camera.lock() = Some(cam);
    // Lepton connection deferred to Task 3+5
    Ok(String::new())
}

#[tauri::command]
pub fn start_stream(
    app: AppHandle,
    state: State<'_, AppState>,
    width: i32,
    height: i32,
    fps: i32,
) -> Result<(), String> {
    let mut cam_guard = state.camera.lock();
    let cam = cam_guard.as_mut().ok_or("Camera not connected")?;

    cam.start_stream(width, height, fps, move |frame_result| {
        let event = FrameEvent {
            data: BASE64.encode(&frame_result.rgba),
            width: frame_result.width,
            height: frame_result.height,
            min_val: frame_result.stats.min_val,
            max_val: frame_result.stats.max_val,
        };
        let _ = app.emit("thermal-frame", event);
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_stream(state: State<'_, AppState>) -> Result<(), String> {
    let mut cam_guard = state.camera.lock();
    if let Some(cam) = cam_guard.as_mut() {
        cam.stop_stream();
    }
    Ok(())
}

#[tauri::command]
pub fn set_palette(state: State<'_, AppState>, palette: String) -> Result<(), String> {
    let cam_guard = state.camera.lock();
    let cam = cam_guard.as_ref().ok_or("Camera not connected")?;
    let p = match palette.as_str() {
        "ironblack" => Palette::IronBlack,
        "rainbow" => Palette::Rainbow,
        "grayscale" => Palette::Grayscale,
        _ => return Err(format!("Unknown palette: {palette}")),
    };
    cam.set_palette(p);
    Ok(())
}

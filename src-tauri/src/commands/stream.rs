use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::camera::acquisition::CameraAcquisition;
use crate::camera::lepton::LeptonController;
use crate::processing::palettes::Palette;
use crate::usb_stream::UsbStream;
use crate::AppState;

#[derive(Clone, Serialize)]
struct FrameEvent {
    data: String,
    width: usize,
    height: usize,
    min_val: u16,
    max_val: u16,
}

#[tauri::command]
pub fn connect_camera(state: State<'_, AppState>) -> Result<String, String> {
    eprintln!("[thermal-v2] Connecting via IOKit USB...");

    let stream = Arc::new(UsbStream::open().map_err(|e| {
        eprintln!("[thermal-v2] USB open failed: {e}");
        e.to_string()
    })?);
    eprintln!("[thermal-v2] USB device opened");

    let cam = CameraAcquisition::new(stream.clone());
    *state.camera.lock() = Some(cam);

    let lepton = Arc::new(LeptonController::new(stream));
    // Force AGC off to preserve raw radiometric Y16 values
    let _ = lepton.set_agc_enable(false);
    let part = lepton.get_part_number().unwrap_or_default();
    eprintln!("[thermal-v2] Lepton controller ready, part: {part}");

    *state.lepton.lock() = Some(lepton);
    Ok(part)
}

#[tauri::command]
pub fn start_stream(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    eprintln!("[thermal-v2] start_stream called");
    let cam_guard = state.camera.lock();
    let cam = cam_guard.as_ref().ok_or("Camera not connected")?;

    cam.start_stream(move |frame_result| {
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
    let cam_guard = state.camera.lock();
    if let Some(cam) = cam_guard.as_ref() {
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

#[tauri::command]
pub fn set_polarity(state: State<'_, AppState>, polarity: u32) -> Result<(), String> {
    let cam_guard = state.camera.lock();
    let cam = cam_guard.as_ref().ok_or("Camera not connected")?;
    cam.set_inverted(polarity != 0);
    Ok(())
}

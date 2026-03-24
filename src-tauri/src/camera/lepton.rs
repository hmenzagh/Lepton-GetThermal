//! Lepton SDK command layer via UVC extension units.
//!
//! The PureThermal board exposes the Lepton's SDK as UVC extension unit controls.
//! Each Lepton module (AGC, SYS, VID, OEM, RAD) maps to a specific extension unit ID,
//! and each command within a module maps to a control ID derived from the command word.

#![allow(dead_code)]

use std::sync::Arc;

use parking_lot::Mutex;

use super::types::CameraError;
use crate::usb_stream::UsbStream;

// ---------------------------------------------------------------------------
// Lepton SDK module IDs (upper bytes of command ID)
// ---------------------------------------------------------------------------
const LEP_CID_AGC_MODULE: u16 = 0x0100;
const LEP_CID_SYS_MODULE: u16 = 0x0200;
const LEP_CID_VID_MODULE: u16 = 0x0300;
const LEP_CID_OEM_MODULE: u16 = 0x0800;
const LEP_CID_RAD_MODULE: u16 = 0x0E00;

// ---------------------------------------------------------------------------
// UVC extension unit IDs for each Lepton module
// ---------------------------------------------------------------------------
const VC_CONTROL_XU_LEP_AGC_ID: u8 = 3;
const VC_CONTROL_XU_LEP_OEM_ID: u8 = 4;
const VC_CONTROL_XU_LEP_RAD_ID: u8 = 5;
const VC_CONTROL_XU_LEP_SYS_ID: u8 = 6;
const VC_CONTROL_XU_LEP_VID_ID: u8 = 7;

// ---------------------------------------------------------------------------
// AGC Module (0x0100)
// ---------------------------------------------------------------------------
pub const LEP_AGC_ENABLE: u16 = 0x0100;
pub const LEP_AGC_POLICY: u16 = 0x0104;
pub const LEP_AGC_HEQ_DAMPENING_FACTOR: u16 = 0x0124;
pub const LEP_AGC_HEQ_MAX_GAIN: u16 = 0x0128;
pub const LEP_AGC_HEQ_CLIP_LIMIT_HIGH: u16 = 0x012C;
pub const LEP_AGC_HEQ_CLIP_LIMIT_LOW: u16 = 0x0130;
pub const LEP_AGC_HEQ_BIN_EXTENSION: u16 = 0x0134;
pub const LEP_AGC_HEQ_MIDPOINT: u16 = 0x0138;
pub const LEP_AGC_HEQ_EMPTY_COUNTS: u16 = 0x013C;
pub const LEP_AGC_HEQ_NORMALIZATION_FACTOR: u16 = 0x0140;
pub const LEP_AGC_HEQ_SCALE_FACTOR: u16 = 0x0144;
pub const LEP_AGC_CALC_ENABLE: u16 = 0x0148;
pub const LEP_AGC_LINEAR_HISTOGRAM_TAIL_SIZE: u16 = 0x014C;
pub const LEP_AGC_LINEAR_HISTOGRAM_CLIP_PERCENT: u16 = 0x0150;
pub const LEP_AGC_LINEAR_MAX_GAIN: u16 = 0x0154;
pub const LEP_AGC_LINEAR_MIDPOINT: u16 = 0x0158;
pub const LEP_AGC_LINEAR_DAMPENING_FACTOR: u16 = 0x015C;

// ---------------------------------------------------------------------------
// SYS Module (0x0200)
// ---------------------------------------------------------------------------
pub const LEP_SYS_FLIR_SERIAL_NUMBER: u16 = 0x0208;
pub const LEP_SYS_FFC_SHUTTER_MODE: u16 = 0x023C;
pub const LEP_SYS_FFC_RUN: u16 = 0x0242; // Run command
pub const LEP_SYS_GAIN_MODE: u16 = 0x0248;

// ---------------------------------------------------------------------------
// VID Module (0x0300)
// ---------------------------------------------------------------------------
pub const LEP_VID_POLARITY: u16 = 0x0300;
pub const LEP_VID_PCOLOR_LUT: u16 = 0x0304;
pub const LEP_VID_SBNUC_ENABLE: u16 = 0x030C;

// ---------------------------------------------------------------------------
// OEM Module (0x0800)
// ---------------------------------------------------------------------------
pub const LEP_OEM_PART_NUMBER: u16 = 0x081C;
pub const LEP_OEM_SW_VERSION_GPP: u16 = 0x0820;
pub const LEP_OEM_SW_VERSION_DSP: u16 = 0x0824;

// ---------------------------------------------------------------------------
// RAD Module (0x0E00)
// ---------------------------------------------------------------------------
pub const LEP_RAD_TLINEAR_RESOLUTION: u16 = 0x0EC4;
pub const LEP_RAD_SPOTMETER_ROI: u16 = 0x0ECC;

// ---------------------------------------------------------------------------
// Command ID mapping functions
// ---------------------------------------------------------------------------

/// Maps a Lepton command ID to the corresponding UVC extension unit ID.
fn command_to_unit_id(command_id: u16) -> Result<u8, CameraError> {
    match command_id & 0x3F00 {
        LEP_CID_AGC_MODULE => Ok(VC_CONTROL_XU_LEP_AGC_ID),
        LEP_CID_SYS_MODULE => Ok(VC_CONTROL_XU_LEP_SYS_ID),
        LEP_CID_VID_MODULE => Ok(VC_CONTROL_XU_LEP_VID_ID),
        LEP_CID_OEM_MODULE => Ok(VC_CONTROL_XU_LEP_OEM_ID),
        LEP_CID_RAD_MODULE => Ok(VC_CONTROL_XU_LEP_RAD_ID),
        other => Err(CameraError::LeptonError(format!(
            "Unknown module ID: 0x{other:04X}"
        ))),
    }
}

/// Computes the UVC control ID from a Lepton command ID.
/// Formula: ((command_id & 0x00FF) >> 2) + 1
fn command_to_control_id(command_id: u16) -> u8 {
    (((command_id & 0x00FF) >> 2) + 1) as u8
}

// ---------------------------------------------------------------------------
// LeptonController
// ---------------------------------------------------------------------------

/// High-level Lepton camera controller.
/// Thread-safe: all operations are serialized via internal mutex.
pub struct LeptonController {
    usb: Arc<UsbStream>,
    lock: Mutex<()>,
}

unsafe impl Send for LeptonController {}
unsafe impl Sync for LeptonController {}

impl LeptonController {
    pub fn new(usb: Arc<UsbStream>) -> Self {
        Self {
            usb,
            lock: Mutex::new(()),
        }
    }

    // ------------------------------------------------------------------
    // Low-level attribute access
    // ------------------------------------------------------------------

    /// Read an attribute from the Lepton as a vector of u16 words.
    pub fn get_attribute(
        &self,
        command_id: u16,
        word_length: usize,
    ) -> Result<Vec<u16>, CameraError> {
        let _guard = self.lock.lock();
        let unit_id = command_to_unit_id(command_id)?;
        let control_id = command_to_control_id(command_id);
        let byte_length = word_length * 2;

        let mut buf = vec![0u8; byte_length];
        self.usb.get_ctrl(unit_id, control_id, &mut buf)?;

        let words: Vec<u16> = buf
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(words)
    }

    /// Write an attribute to the Lepton as a slice of u16 words.
    pub fn set_attribute(&self, command_id: u16, data: &[u16]) -> Result<(), CameraError> {
        let _guard = self.lock.lock();
        let unit_id = command_to_unit_id(command_id)?;
        let control_id = command_to_control_id(command_id);

        let buf: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.usb.set_ctrl(unit_id, control_id, &buf)?;
        Ok(())
    }

    /// Convenience: read a single u16 attribute.
    pub fn get_u16(&self, command_id: u16) -> Result<u16, CameraError> {
        let words = self.get_attribute(command_id, 1)?;
        Ok(words[0])
    }

    /// Convenience: write a single u16 attribute.
    pub fn set_u16(&self, command_id: u16, value: u16) -> Result<(), CameraError> {
        self.set_attribute(command_id, &[value])
    }

    /// Execute a run command by sending the control_id as a single byte,
    /// matching the PureThermal firmware protocol.
    pub fn run_command(&self, command_id: u16) -> Result<(), CameraError> {
        let _guard = self.lock.lock();
        let unit_id = command_to_unit_id(command_id)?;
        let control_id = command_to_control_id(command_id);
        self.usb.set_ctrl(unit_id, control_id, &[control_id])?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // AGC Module
    // ------------------------------------------------------------------

    /// Get AGC enable state. Returns true if AGC is enabled.
    pub fn get_agc_enable(&self) -> Result<bool, CameraError> {
        Ok(self.get_u16(LEP_AGC_ENABLE)? != 0)
    }

    /// Enable or disable AGC.
    pub fn set_agc_enable(&self, enable: bool) -> Result<(), CameraError> {
        self.set_u16(LEP_AGC_ENABLE, if enable { 1 } else { 0 })
    }

    /// Get AGC policy (0 = linear, 1 = HEQ).
    pub fn get_agc_policy(&self) -> Result<u16, CameraError> {
        self.get_u16(LEP_AGC_POLICY)
    }

    /// Set AGC policy (0 = linear, 1 = HEQ).
    pub fn set_agc_policy(&self, policy: u16) -> Result<(), CameraError> {
        self.set_u16(LEP_AGC_POLICY, policy)
    }

    // ------------------------------------------------------------------
    // SYS Module
    // ------------------------------------------------------------------

    /// Get FFC shutter mode (0 = manual, 1 = auto).
    pub fn get_ffc_mode(&self) -> Result<u16, CameraError> {
        self.get_u16(LEP_SYS_FFC_SHUTTER_MODE)
    }

    /// Set FFC shutter mode (0 = manual, 1 = auto).
    pub fn set_ffc_mode(&self, mode: u16) -> Result<(), CameraError> {
        self.set_u16(LEP_SYS_FFC_SHUTTER_MODE, mode)
    }

    /// Trigger a flat-field correction (FFC / shutter recalibration).
    pub fn perform_ffc(&self) -> Result<(), CameraError> {
        self.run_command(LEP_SYS_FFC_RUN)
    }

    /// Get gain mode (0 = high, 1 = low, 2 = auto).
    pub fn get_gain_mode(&self) -> Result<u16, CameraError> {
        self.get_u16(LEP_SYS_GAIN_MODE)
    }

    /// Set gain mode (0 = high, 1 = low, 2 = auto).
    pub fn set_gain_mode(&self, mode: u16) -> Result<(), CameraError> {
        self.set_u16(LEP_SYS_GAIN_MODE, mode)
    }

    /// Get the FLIR serial number (4 words = 64-bit serial).
    pub fn get_serial_number(&self) -> Result<u64, CameraError> {
        let words = self.get_attribute(LEP_SYS_FLIR_SERIAL_NUMBER, 4)?;
        let serial = (words[3] as u64) << 48
            | (words[2] as u64) << 32
            | (words[1] as u64) << 16
            | (words[0] as u64);
        Ok(serial)
    }

    // ------------------------------------------------------------------
    // VID Module
    // ------------------------------------------------------------------

    /// Get video polarity (0 = white-hot, 1 = black-hot).
    pub fn get_polarity(&self) -> Result<u16, CameraError> {
        self.get_u16(LEP_VID_POLARITY)
    }

    /// Set video polarity (0 = white-hot, 1 = black-hot).
    pub fn set_polarity(&self, polarity: u16) -> Result<(), CameraError> {
        self.set_u16(LEP_VID_POLARITY, polarity)
    }

    /// Get pseudo-color LUT index.
    pub fn get_pcolor_lut(&self) -> Result<u16, CameraError> {
        self.get_u16(LEP_VID_PCOLOR_LUT)
    }

    /// Set pseudo-color LUT index.
    pub fn set_pcolor_lut(&self, lut: u16) -> Result<(), CameraError> {
        self.set_u16(LEP_VID_PCOLOR_LUT, lut)
    }

    // ------------------------------------------------------------------
    // RAD Module
    // ------------------------------------------------------------------

    /// Get spotmeter ROI as (startRow, startCol, endRow, endCol).
    pub fn get_spotmeter_roi(&self) -> Result<[u16; 4], CameraError> {
        let words = self.get_attribute(LEP_RAD_SPOTMETER_ROI, 4)?;
        Ok([words[0], words[1], words[2], words[3]])
    }

    /// Set spotmeter ROI as (startRow, startCol, endRow, endCol).
    pub fn set_spotmeter_roi(&self, roi: [u16; 4]) -> Result<(), CameraError> {
        self.set_attribute(LEP_RAD_SPOTMETER_ROI, &roi)
    }

    /// Get TLinear resolution (0 = 0.1K, 1 = 0.01K).
    pub fn get_tlinear_resolution(&self) -> Result<u16, CameraError> {
        self.get_u16(LEP_RAD_TLINEAR_RESOLUTION)
    }

    // ------------------------------------------------------------------
    // OEM Module
    // ------------------------------------------------------------------

    /// Get the OEM part number as a string.
    pub fn get_part_number(&self) -> Result<String, CameraError> {
        // Part number is 16 words (32 bytes), ASCII string
        let words = self.get_attribute(LEP_OEM_PART_NUMBER, 16)?;
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let s = String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_string();
        Ok(s)
    }

    /// Check if the Lepton supports radiometry by attempting to read the
    /// TLinear resolution. If the read succeeds, radiometry is supported.
    pub fn supports_radiometry(&self) -> bool {
        self.get_tlinear_resolution().is_ok()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agc_module_maps_to_unit_3() {
        assert_eq!(command_to_unit_id(0x0100).unwrap(), 3);
        assert_eq!(command_to_unit_id(0x0104).unwrap(), 3);
    }

    #[test]
    fn sys_module_maps_to_unit_6() {
        assert_eq!(command_to_unit_id(0x0200).unwrap(), 6);
    }

    #[test]
    fn vid_module_maps_to_unit_7() {
        assert_eq!(command_to_unit_id(0x0300).unwrap(), 7);
    }

    #[test]
    fn oem_module_maps_to_unit_4() {
        assert_eq!(command_to_unit_id(0x0800).unwrap(), 4);
    }

    #[test]
    fn rad_module_maps_to_unit_5() {
        assert_eq!(command_to_unit_id(0x0E00).unwrap(), 5);
    }

    #[test]
    fn unknown_module_returns_error() {
        assert!(command_to_unit_id(0x9900).is_err());
    }

    #[test]
    fn control_id_calculation() {
        // command 0x0100: (0x00 >> 2) + 1 = 1
        assert_eq!(command_to_control_id(0x0100), 1);
        // command 0x0104: (0x04 >> 2) + 1 = 2
        assert_eq!(command_to_control_id(0x0104), 2);
        // command 0x0148: (0x48 >> 2) + 1 = 19
        assert_eq!(command_to_control_id(0x0148), 19);
    }

    #[test]
    fn all_agc_commands_map_to_unit_3() {
        for cmd in [
            LEP_AGC_ENABLE,
            LEP_AGC_POLICY,
            LEP_AGC_HEQ_SCALE_FACTOR,
            LEP_AGC_CALC_ENABLE,
        ] {
            assert_eq!(
                command_to_unit_id(cmd).unwrap(),
                VC_CONTROL_XU_LEP_AGC_ID,
                "cmd 0x{cmd:04X}"
            );
        }
    }

    #[test]
    fn all_vid_commands_map_to_unit_7() {
        for cmd in [LEP_VID_POLARITY, LEP_VID_PCOLOR_LUT, LEP_VID_SBNUC_ENABLE] {
            assert_eq!(
                command_to_unit_id(cmd).unwrap(),
                VC_CONTROL_XU_LEP_VID_ID,
                "cmd 0x{cmd:04X}"
            );
        }
    }

    #[test]
    fn rad_spotmeter_maps_to_unit_5() {
        assert_eq!(
            command_to_unit_id(LEP_RAD_SPOTMETER_ROI).unwrap(),
            VC_CONTROL_XU_LEP_RAD_ID
        );
    }

    #[test]
    fn oem_part_number_maps_to_unit_4() {
        assert_eq!(
            command_to_unit_id(LEP_OEM_PART_NUMBER).unwrap(),
            VC_CONTROL_XU_LEP_OEM_ID
        );
    }

    #[test]
    fn control_id_for_known_commands() {
        // LEP_AGC_ENABLE = 0x0100 -> control 1
        assert_eq!(command_to_control_id(LEP_AGC_ENABLE), 1);
        // LEP_AGC_POLICY = 0x0104 -> control 2
        assert_eq!(command_to_control_id(LEP_AGC_POLICY), 2);
        // LEP_VID_POLARITY = 0x0300 -> control 1
        assert_eq!(command_to_control_id(LEP_VID_POLARITY), 1);
        // LEP_VID_PCOLOR_LUT = 0x0304 -> control 2
        assert_eq!(command_to_control_id(LEP_VID_PCOLOR_LUT), 2);
        // LEP_SYS_GAIN_MODE = 0x0248 -> (0x48 >> 2) + 1 = 19
        assert_eq!(command_to_control_id(LEP_SYS_GAIN_MODE), 19);
    }
}

pub mod autogain;
pub mod colorize;
pub mod palettes;
pub mod upscale;

use autogain::GainResult;
use palettes::Palette;

/// Result of processing a single Y16 frame.
#[derive(Debug, Clone)]
pub struct FrameResult {
    /// RGBA pixel data ready for display
    pub rgba: Vec<u8>,
    /// Frame width in pixels (may be upscaled)
    pub width: usize,
    /// Frame height in pixels (may be upscaled)
    pub height: usize,
    /// Processing statistics (always in original sensor coordinates)
    pub stats: GainResult,
}

/// Full processing pipeline: Y16 → auto-gain → colorize → RGBA → isotherm overlay.
/// When `invert` is true, the grayscale is flipped (white-hot ↔ black-hot).
/// When `isotherm_raw` > 0, pixels whose raw Y16 value exceeds the threshold
/// get a diagonal red/white stripe pattern.
/// When `upscaler` is provided, the grayscale is super-resolved before colorization.
pub fn process_frame(
    y16_data: &[u8],
    width: usize,
    height: usize,
    palette: Palette,
    invert: bool,
    isotherm_raw: u16,
    upscaler: Option<&mut upscale::Upscaler>,
) -> FrameResult {
    let mut stats = autogain::auto_gain(y16_data, width, height);
    if invert {
        for val in stats.grayscale.iter_mut() {
            *val = 255 - *val;
        }
    }

    // Upscale grayscale if a model is available
    let upscaled = upscaler.and_then(|up| {
        match up.upscale(&stats.grayscale, width, height) {
            Ok(result) => Some(result),
            Err(e) => {
                eprintln!("[upscale] inference failed: {e}");
                None
            }
        }
    });

    let (display_gs, display_w, display_h) = match &upscaled {
        Some((gs, w, h)) => (gs.as_slice(), *w, *h),
        None => (stats.grayscale.as_slice(), width, height),
    };

    let mut rgba = colorize::colorize(display_gs, palette);

    // Apply isotherm overlay: diagonal red/white stripes blended over thermal image.
    // Maps upscaled pixel coordinates back to original Y16 data for threshold check.
    if isotherm_raw > 0 {
        let pixel_count = display_w * display_h;
        for i in 0..pixel_count {
            let row = i / display_w;
            let col = i % display_w;
            // Map to original Y16 coordinates (integer division works for integer scale factors)
            let orig_row = (row * height) / display_h;
            let orig_col = (col * width) / display_w;
            let orig_i = orig_row.min(height - 1) * width + orig_col.min(width - 1);
            let raw = u16::from_le_bytes([y16_data[orig_i * 2], y16_data[orig_i * 2 + 1]]);
            if raw >= isotherm_raw {
                let stripe = ((row + col) % 2) == 0;
                let base = i * 4;
                let (sr, sg, sb): (u16, u16, u16) =
                    if stripe { (220, 30, 30) } else { (255, 255, 255) };
                // Blend at 75% stripe / 25% original
                rgba[base] = ((sr * 192 + rgba[base] as u16 * 64) >> 8) as u8;
                rgba[base + 1] = ((sg * 192 + rgba[base + 1] as u16 * 64) >> 8) as u8;
                rgba[base + 2] = ((sb * 192 + rgba[base + 2] as u16 * 64) >> 8) as u8;
            }
        }
    }

    FrameResult {
        rgba,
        width: display_w,
        height: display_h,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_y16_frame(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn pipeline_produces_correct_rgba_output() {
        // 2x2 frame, values 0-300
        let frame = make_y16_frame(&[0, 100, 200, 300]);
        let result = process_frame(&frame, 2, 2, Palette::Grayscale, false, 0, None);

        assert_eq!(result.rgba.len(), 16); // 4 pixels * 4 RGBA
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
        assert_eq!(result.rgba[3], 255); // alpha
        assert_eq!(result.rgba[12], 255); // R
        assert_eq!(result.rgba[15], 255); // A
    }

    #[test]
    fn isotherm_applies_stripes() {
        let frame = make_y16_frame(&[100, 200, 300, 400]);
        let result_no_iso = process_frame(&frame, 2, 2, Palette::Grayscale, false, 0, None);
        let result_iso = process_frame(&frame, 2, 2, Palette::Grayscale, false, 250, None);

        // Pixels 0,1 (100,200) below threshold → unchanged
        assert_eq!(&result_iso.rgba[0..8], &result_no_iso.rgba[0..8]);
        // Pixels 2,3 (300,400) above threshold → blended, should differ from original
        assert_ne!(&result_iso.rgba[8..12], &result_no_iso.rgba[8..12]);
        assert_ne!(&result_iso.rgba[12..16], &result_no_iso.rgba[12..16]);
    }
}

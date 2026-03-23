pub mod autogain;
pub mod colorize;
pub mod palettes;

use autogain::GainResult;
use palettes::Palette;

/// Result of processing a single Y16 frame.
#[derive(Debug, Clone)]
pub struct FrameResult {
    /// RGBA pixel data ready for display
    pub rgba: Vec<u8>,
    /// Frame width in pixels
    pub width: usize,
    /// Frame height in pixels
    pub height: usize,
    /// Processing statistics
    pub stats: GainResult,
}

/// Full processing pipeline: Y16 → auto-gain → colorize → RGBA → isotherm overlay.
/// When `invert` is true, the grayscale is flipped (white-hot ↔ black-hot).
/// When `isotherm_raw` > 0, pixels whose raw Y16 value exceeds the threshold
/// get a diagonal red/white stripe pattern.
pub fn process_frame(
    y16_data: &[u8],
    width: usize,
    height: usize,
    palette: Palette,
    invert: bool,
    isotherm_raw: u16,
) -> FrameResult {
    let mut stats = autogain::auto_gain(y16_data, width, height);
    if invert {
        for val in stats.grayscale.iter_mut() {
            *val = 255 - *val;
        }
    }
    let mut rgba = colorize::colorize(&stats.grayscale, palette);

    // Apply isotherm overlay: diagonal red/white stripes blended over thermal image
    if isotherm_raw > 0 {
        let pixel_count = width * height;
        for i in 0..pixel_count {
            let raw = u16::from_le_bytes([y16_data[i * 2], y16_data[i * 2 + 1]]);
            if raw >= isotherm_raw {
                let row = i / width;
                let col = i % width;
                let stripe = ((row + col) % 2) == 0;
                let base = i * 4;
                // Stripe color: red or white
                let (sr, sg, sb): (u16, u16, u16) = if stripe { (220, 30, 30) } else { (255, 255, 255) };
                // Blend at 75% stripe / 25% original
                rgba[base]     = ((sr * 192 + rgba[base] as u16 * 64) >> 8) as u8;
                rgba[base + 1] = ((sg * 192 + rgba[base + 1] as u16 * 64) >> 8) as u8;
                rgba[base + 2] = ((sb * 192 + rgba[base + 2] as u16 * 64) >> 8) as u8;
            }
        }
    }

    FrameResult {
        rgba,
        width,
        height,
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
        let result = process_frame(&frame, 2, 2, Palette::Grayscale, false, 0);

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
        let result_no_iso = process_frame(&frame, 2, 2, Palette::Grayscale, false, 0);
        let result_iso = process_frame(&frame, 2, 2, Palette::Grayscale, false, 250);

        // Pixels 0,1 (100,200) below threshold → unchanged
        assert_eq!(&result_iso.rgba[0..8], &result_no_iso.rgba[0..8]);
        // Pixels 2,3 (300,400) above threshold → blended, should differ from original
        assert_ne!(&result_iso.rgba[8..12], &result_no_iso.rgba[8..12]);
        assert_ne!(&result_iso.rgba[12..16], &result_no_iso.rgba[12..16]);
    }
}

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

/// Full processing pipeline: Y16 → auto-gain → colorize → RGBA.
pub fn process_frame(
    y16_data: &[u8],
    width: usize,
    height: usize,
    palette: Palette,
) -> FrameResult {
    let stats = autogain::auto_gain(y16_data, width, height);
    let rgba = colorize::colorize(&stats.grayscale, palette);
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
        let result = process_frame(&frame, 2, 2, Palette::Grayscale);

        assert_eq!(result.rgba.len(), 16); // 4 pixels * 4 RGBA
        assert_eq!(result.width, 2);
        assert_eq!(result.height, 2);
        // First pixel (min=0): grayscale 0 → RGBA(0,0,0,255)
        assert_eq!(result.rgba[3], 255); // alpha
        // Last pixel (max=300): grayscale 255 → RGBA(255,255,255,255)
        assert_eq!(result.rgba[12], 255); // R
        assert_eq!(result.rgba[15], 255); // A
    }
}

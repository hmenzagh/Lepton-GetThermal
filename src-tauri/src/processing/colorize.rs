use super::palettes::{get_palette, Palette};

/// Applies a color palette LUT to an 8-bit grayscale buffer.
/// Returns an RGBA buffer (4 bytes per pixel, alpha = 255).
pub fn colorize(grayscale: &[u8], palette: Palette) -> Vec<u8> {
    let lut = get_palette(palette);
    let mut rgba = Vec::with_capacity(grayscale.len() * 4);
    for &val in grayscale {
        let idx = val as usize * 3;
        rgba.push(lut[idx]);     // R
        rgba.push(lut[idx + 1]); // G
        rgba.push(lut[idx + 2]); // B
        rgba.push(255);          // A
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorize_maps_grayscale_to_rgba() {
        let input = vec![0u8, 128, 255];
        let result = colorize(&input, Palette::Grayscale);

        assert_eq!(result.len(), 12); // 3 pixels * 4 bytes RGBA
        // Pixel 0: grayscale value 0 -> (0, 0, 0, 255)
        assert_eq!(&result[0..4], &[0, 0, 0, 255]);
        // Pixel 1: grayscale value 128 -> (128, 128, 128, 255)
        assert_eq!(&result[4..8], &[128, 128, 128, 255]);
        // Pixel 2: grayscale value 255 -> (255, 255, 255, 255)
        assert_eq!(&result[8..12], &[255, 255, 255, 255]);
    }

    #[test]
    fn colorize_empty_input() {
        let result = colorize(&[], Palette::IronBlack);
        assert!(result.is_empty());
    }
}

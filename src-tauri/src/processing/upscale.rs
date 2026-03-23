//! Neural network super-resolution upscaling for thermal frames.
//! Uses ONNX Runtime to run an ESPCN/IMDN model on grayscale data.

use ndarray::Array4;
use ort::session::Session;
use ort::value::TensorRef;

/// Embedded ESPCN 3x model from ONNX Model Zoo (240KB).
/// Accepts single-channel float32 input normalized to [0,1].
/// Fixed input shape: [1, 1, 224, 224]. We pad and crop accordingly.
const MODEL_BYTES: &[u8] = include_bytes!("../../models/super_resolution.onnx");

/// Expected input dimensions of the embedded ESPCN model.
const MODEL_INPUT_H: usize = 224;
const MODEL_INPUT_W: usize = 224;

pub struct Upscaler {
    session: Session,
    pub scale: usize,
}

unsafe impl Send for Upscaler {}
unsafe impl Sync for Upscaler {}

impl Upscaler {
    /// Load the embedded ONNX super-resolution model.
    pub fn new() -> Result<Self, String> {
        let mut session = Session::builder()
            .map_err(|e| format!("ORT session builder: {e}"))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| format!("ORT optimization: {e}"))?
            .commit_from_memory(MODEL_BYTES)
            .map_err(|e| format!("ORT load model: {e}"))?;

        let scale = Self::detect_scale(&mut session)?;
        eprintln!("[upscale] Model loaded, scale factor: {scale}x");

        Ok(Self { session, scale })
    }

    /// Load a custom ONNX model from a file path (e.g. thermal IMDN).
    pub fn from_file(path: &str) -> Result<Self, String> {
        let mut session = Session::builder()
            .map_err(|e| format!("ORT session builder: {e}"))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| format!("ORT optimization: {e}"))?
            .commit_from_file(path)
            .map_err(|e| format!("ORT load model: {e}"))?;

        let scale = Self::detect_scale(&mut session)?;
        eprintln!("[upscale] Custom model loaded from {path}, scale: {scale}x");

        Ok(Self { session, scale })
    }

    /// Detect the upscale factor by running inference with the model's expected input size.
    fn detect_scale(session: &mut Session) -> Result<usize, String> {
        let input = Array4::<f32>::zeros((1, 1, MODEL_INPUT_H, MODEL_INPUT_W));
        let tensor = TensorRef::from_array_view(input.view())
            .map_err(|e| format!("Scale detect tensor: {e}"))?;
        let outputs = session
            .run(ort::inputs![tensor])
            .map_err(|e| format!("Scale detect inference: {e}"))?;
        let (shape, _data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Scale detect extract: {e}"))?;
        let out_h = shape[2] as usize;
        let scale = out_h / MODEL_INPUT_H;
        eprintln!("[upscale] detect_scale: input {MODEL_INPUT_H}x{MODEL_INPUT_W} -> output {}x{}, scale={scale}x", shape[2], shape[3]);
        Ok(scale)
    }

    /// Upscale a grayscale buffer.
    /// Pads input to model dimensions (224x224), runs inference, then crops output.
    pub fn upscale(
        &mut self,
        grayscale: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(Vec<u8>, usize, usize), String> {
        // Build padded input [1, 1, MODEL_H, MODEL_W] with edge replication
        let input = Array4::<f32>::from_shape_fn(
            (1, 1, MODEL_INPUT_H, MODEL_INPUT_W),
            |(_, _, y, x)| {
                let sy = y.min(height - 1);
                let sx = x.min(width - 1);
                grayscale[sy * width + sx] as f32 / 255.0
            },
        );

        let tensor = TensorRef::from_array_view(input.view())
            .map_err(|e| format!("ORT tensor: {e}"))?;

        let outputs = self
            .session
            .run(ort::inputs![tensor])
            .map_err(|e| format!("ORT inference: {e}"))?;

        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("ORT extract: {e}"))?;

        let full_out_w = shape[3] as usize;

        // Crop to original dimensions * scale
        let out_h = height * self.scale;
        let out_w = width * self.scale;

        let mut result = Vec::with_capacity(out_h * out_w);
        for y in 0..out_h {
            for x in 0..out_w {
                let v = data[y * full_out_w + x];
                result.push((v.clamp(0.0, 1.0) * 255.0) as u8);
            }
        }

        Ok((result, out_w, out_h))
    }
}

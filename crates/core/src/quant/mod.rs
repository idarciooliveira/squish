//! Palette quantizers behind one trait (ADR-0004, ADR-0022).

#[cfg(feature = "quant-quantizr")]
mod quantizr;

/// A palette and one index per pixel.
pub struct Quantized {
    pub palette: Vec<[u8; 4]>,
    pub indices: Vec<u8>,
}

pub trait Quantizer: Sync {
    /// Reduces `rgba` to at most `max_colors` colors with error-diffusion dithering of
    /// strength `dithering` (0.0 to 1.0). The quality search lives outside the trait (ADR-0024).
    fn quantize(&self, rgba: &[u8], width: u32, height: u32, max_colors: u16, dithering: f32) -> Option<Quantized>;
}

/// The quantizer compiled in, or `None` when every `quant-*` feature is off.
pub fn default_quantizer() -> Option<&'static dyn Quantizer> {
    #[cfg(feature = "quant-quantizr")]
    {
        Some(&quantizr::Quantizr)
    }
    #[cfg(not(feature = "quant-quantizr"))]
    {
        None
    }
}

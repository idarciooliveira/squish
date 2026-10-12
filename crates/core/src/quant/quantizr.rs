//! quantizr (MIT): RGBA input, dithering, no quality target of its own (ADR-0022).

use super::{Quantized, Quantizer};

pub(super) struct Quantizr;

impl Quantizer for Quantizr {
    fn quantize(&self, rgba: &[u8], width: u32, height: u32, max_colors: u16, dithering: f32) -> Option<Quantized> {
        let (w, h) = (width as usize, height as usize);
        let image = ::quantizr::Image::new(rgba, w, h).ok()?;
        let mut options = ::quantizr::Options::default();
        options.set_max_colors(i32::from(max_colors.clamp(2, 256))).ok()?;
        let mut result = ::quantizr::QuantizeResult::quantize(&image, &options);
        result.set_dithering_level(dithering.clamp(0.0, 1.0)).ok()?;
        let mut indices = vec![0u8; w * h];
        result.remap_image(&image, &mut indices).ok()?;
        let palette = result.get_palette();
        let palette = palette.entries[..palette.count as usize].iter().map(|c| [c.r, c.g, c.b, c.a]).collect();
        Some(Quantized { palette, indices })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_the_color_limit_and_keeps_alpha() {
        let (w, h) = (32u32, 32u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[
                    (x * 8) as u8,
                    (y * 8) as u8,
                    ((x + y) * 4) as u8,
                    if x < 8 { 0 } else { 255 },
                ]);
            }
        }
        let q = Quantizr.quantize(&rgba, w, h, 16, 1.0).unwrap();
        assert!(q.palette.len() <= 16);
        assert_eq!(q.indices.len(), (w * h) as usize);
        assert!(q.indices.iter().all(|&i| usize::from(i) < q.palette.len()));
        // Transparent pixels stay transparent.
        assert!(q.indices.chunks(w as usize).all(|row| row[..8].iter().all(|&i| q.palette[usize::from(i)][3] < 128)));
    }
}

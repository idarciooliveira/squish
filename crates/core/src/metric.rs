//! The quality metric (ADR-0026): fast-ssim2 on a copy box-downscaled to about 1 MP.
//!
//! Images with alpha are composited over white and over black, and the lower score
//! counts. Scores are on fast-ssim2's scale, 100 for identical images.

use fast_ssim2::{PixelDescriptor, PixelSlice, Ssimulacra2Reference};

use crate::error::Error;

/// The scoring copy has at most about this many pixels.
const SCORE_PIXELS: f64 = 1.0e6;

/// How the bytes of a pixel buffer are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    Gray,
    Rgb,
    Rgba,
}

impl Layout {
    pub(crate) fn channels(self) -> usize {
        match self {
            Layout::Gray => 1,
            Layout::Rgb => 3,
            Layout::Rgba => 4,
        }
    }
}

/// Integer downscale factor that brings an image to at most about 1 MP.
pub(crate) fn factor_for(width: u32, height: u32) -> usize {
    let megapixels = f64::from(width) * f64::from(height) / SCORE_PIXELS;
    (megapixels.sqrt().ceil() as usize).max(1)
}

/// A downscaled RGB copy of each background the image is judged over.
struct Views {
    width: usize,
    height: usize,
    /// One buffer for opaque images, two (white, black) when any pixel has alpha.
    rgb: Vec<Vec<u8>>,
}

/// Box-averages `factor`x`factor` blocks and composites alpha in the same pass.
///
/// Averaging premultiplied values and then compositing gives the same result as
/// compositing every pixel and then averaging, so one pass produces both views.
fn views(pixels: &[u8], width: usize, height: usize, layout: Layout, factor: usize, has_alpha: bool) -> Views {
    let out_w = (width / factor).max(1);
    let out_h = (height / factor).max(1);
    let backgrounds: &[u32] = if has_alpha { &[255, 0] } else { &[0] };
    let mut rgb: Vec<Vec<u8>> = backgrounds.iter().map(|_| vec![0u8; out_w * out_h * 3]).collect();
    let channels = layout.channels();
    let n = (factor * factor) as u32;

    for oy in 0..out_h {
        for ox in 0..out_w {
            let mut sum = [0u32; 3];
            let mut alpha_sum = 0u32;
            for dy in 0..factor {
                let y = (oy * factor + dy).min(height - 1);
                for dx in 0..factor {
                    let x = (ox * factor + dx).min(width - 1);
                    let i = (y * width + x) * channels;
                    let (r, g, b, a) = match layout {
                        Layout::Gray => (pixels[i], pixels[i], pixels[i], 255),
                        Layout::Rgb => (pixels[i], pixels[i + 1], pixels[i + 2], 255),
                        Layout::Rgba => (pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]),
                    };
                    let a = u32::from(a);
                    if has_alpha {
                        sum[0] += u32::from(r) * a;
                        sum[1] += u32::from(g) * a;
                        sum[2] += u32::from(b) * a;
                        alpha_sum += a;
                    } else {
                        sum[0] += u32::from(r);
                        sum[1] += u32::from(g);
                        sum[2] += u32::from(b);
                    }
                }
            }
            let o = (oy * out_w + ox) * 3;
            if has_alpha {
                let full = 255 * n;
                for (view, &background) in rgb.iter_mut().zip(backgrounds) {
                    for c in 0..3 {
                        view[o + c] = ((sum[c] + background * (full - alpha_sum) + full / 2) / full) as u8;
                    }
                }
            } else {
                for c in 0..3 {
                    rgb[0][o + c] = ((sum[c] + n / 2) / n) as u8;
                }
            }
        }
    }
    Views { width: out_w, height: out_h, rgb }
}

fn slice(data: &[u8], width: usize, height: usize) -> Result<PixelSlice<'_>, Error> {
    PixelSlice::new(data, width as u32, height as u32, width * 3, PixelDescriptor::RGB8_SRGB)
        .map_err(|e| Error::Encode(format!("quality metric: {e}")))
}

/// The original image, prepared once and compared against each candidate.
pub(crate) struct Reference {
    references: Vec<Ssimulacra2Reference>,
    width: usize,
    height: usize,
    layout: Layout,
    factor: usize,
    has_alpha: bool,
}

impl Reference {
    pub(crate) fn new(pixels: &[u8], width: u32, height: u32, layout: Layout) -> Result<Self, Error> {
        let (w, h) = (width as usize, height as usize);
        debug_assert_eq!(pixels.len(), w * h * layout.channels());
        let has_alpha = layout == Layout::Rgba && pixels.chunks_exact(4).any(|p| p[3] != 255);
        let factor = factor_for(width, height);
        let scaled = views(pixels, w, h, layout, factor, has_alpha);
        let references = scaled
            .rgb
            .iter()
            .map(|v| {
                let s = slice(v, scaled.width, scaled.height)?;
                Ssimulacra2Reference::new(&s).map_err(|e| Error::Encode(format!("quality metric: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Reference { references, width: w, height: h, layout, factor, has_alpha })
    }

    /// Score of `pixels`, which must match the reference's size and layout.
    pub(crate) fn score(&self, pixels: &[u8]) -> Result<f64, Error> {
        debug_assert_eq!(pixels.len(), self.width * self.height * self.layout.channels());
        let scaled = views(pixels, self.width, self.height, self.layout, self.factor, self.has_alpha);
        let mut lowest = f64::INFINITY;
        for (reference, view) in self.references.iter().zip(&scaled.rgb) {
            let s = slice(view, scaled.width, scaled.height)?;
            let score = reference.compare(&s).map_err(|e| Error::Encode(format!("quality metric: {e}")))?;
            lowest = lowest.min(score);
        }
        Ok(lowest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: usize, height: usize) -> Vec<u8> {
        (0..width * height * 3).map(|i| ((i / 3 % width) * 255 / width) as u8 ^ (i % 3 * 40) as u8).collect()
    }

    #[test]
    fn identical_images_score_100() {
        let px = gradient(64, 48);
        let reference = Reference::new(&px, 64, 48, Layout::Rgb).unwrap();
        assert_eq!(reference.score(&px).unwrap(), 100.0);
    }

    #[test]
    fn damage_lowers_the_score_in_proportion() {
        let px = gradient(96, 96);
        let reference = Reference::new(&px, 96, 96, Layout::Rgb).unwrap();
        let noisy = |amount: u8| -> Vec<u8> {
            px.iter().enumerate().map(|(i, &v)| if i % 7 < 3 { v.saturating_add(amount) } else { v }).collect()
        };
        let mild = reference.score(&noisy(4)).unwrap();
        let heavy = reference.score(&noisy(60)).unwrap();
        assert!(mild < 100.0 && heavy < mild, "mild {mild}, heavy {heavy}");
    }

    #[test]
    fn tiny_images_are_scored() {
        let px = vec![200u8; 3 * 3 * 3];
        let reference = Reference::new(&px, 3, 3, Layout::Rgb).unwrap();
        assert_eq!(reference.score(&px).unwrap(), 100.0);
    }

    #[test]
    fn large_images_are_scored_at_about_one_megapixel() {
        assert_eq!(factor_for(1000, 1000), 1);
        assert_eq!(factor_for(1001, 1000), 2);
        assert_eq!(factor_for(4000, 3000), 4);
        assert_eq!(factor_for(7000, 7000), 7);
    }

    #[test]
    fn alpha_is_judged_over_white_and_black() {
        // A red square, fully transparent in the left half. Changing the hidden RGB
        // under transparent pixels must not change the score.
        let (w, h) = (32, 32);
        let mut a: Vec<u8> = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                a.extend_from_slice(&if x < w / 2 { [0, 0, 0, 0] } else { [255, 0, 0, 255] });
            }
        }
        let reference = Reference::new(&a, w as u32, h as u32, Layout::Rgba).unwrap();
        let mut b = a.clone();
        for p in b.chunks_exact_mut(4).filter(|p| p[3] == 0) {
            p[0] = 90;
            p[1] = 200;
        }
        assert_eq!(reference.score(&b).unwrap(), 100.0);

        // Making the visible half transparent is a real change.
        let mut c = a.clone();
        for p in c.chunks_exact_mut(4) {
            p[3] = 0;
        }
        assert!(reference.score(&c).unwrap() < 50.0);
    }

    #[test]
    fn opaque_rgba_matches_rgb() {
        let rgb = gradient(40, 40);
        let rgba: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let noisy_rgb: Vec<u8> = rgb.iter().map(|&v| v.saturating_sub(9)).collect();
        let noisy_rgba: Vec<u8> = noisy_rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let a = Reference::new(&rgb, 40, 40, Layout::Rgb).unwrap().score(&noisy_rgb).unwrap();
        let b = Reference::new(&rgba, 40, 40, Layout::Rgba).unwrap().score(&noisy_rgba).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn gray_is_scored_as_equal_channels() {
        let gray: Vec<u8> = (0..30 * 30).map(|i| (i % 251) as u8).collect();
        let rgb: Vec<u8> = gray.iter().flat_map(|&v| [v, v, v]).collect();
        let dim_gray: Vec<u8> = gray.iter().map(|&v| v / 2).collect();
        let dim_rgb: Vec<u8> = rgb.iter().map(|&v| v / 2).collect();
        let a = Reference::new(&gray, 30, 30, Layout::Gray).unwrap().score(&dim_gray).unwrap();
        let b = Reference::new(&rgb, 30, 30, Layout::Rgb).unwrap().score(&dim_rgb).unwrap();
        assert_eq!(a, b);
    }
}

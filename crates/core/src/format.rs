//! Format detection from magic bytes, animation checks and header dimensions.
//!
//! Nothing here decodes pixels. Everything reads a few header bytes, so it is cheap
//! enough to run before the pixel limit check.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    WebP,
    Avif,
}

impl Format {
    /// Lowercase name used in reports: `png`, `jpeg`, `webp`, `avif`.
    pub fn name(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpeg",
            Format::WebP => "webp",
            Format::Avif => "avif",
        }
    }

    /// The media type for `Content-Type`.
    pub fn mime_type(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg => "image/jpeg",
            Format::WebP => "image/webp",
            Format::Avif => "image/avif",
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Format::Png => "PNG",
            Format::Jpeg => "JPEG",
            Format::WebP => "WebP",
            Format::Avif => "AVIF",
        })
    }
}

/// What the header says about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub format: Format,
    pub animated: bool,
    /// `None` when the header is damaged, or for AVIF, whose header isn't parsed yet.
    pub dimensions: Option<(u32, u32)>,
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Identifies PNG, JPEG, WebP and AVIF from their magic bytes. GIF and everything else give `None`.
pub fn detect(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(PNG_SIGNATURE) {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else if is_avif(bytes) {
        Some(Format::Avif)
    } else {
        None
    }
}

/// Detects the format and reads the header.
pub fn inspect(bytes: &[u8]) -> Option<Info> {
    let format = detect(bytes)?;
    Some(Info { format, animated: is_animated(bytes, format), dimensions: dimensions(bytes, format) })
}

/// True for APNG, animated WebP and AVIF image sequences. JPEG is never animated.
pub fn is_animated(bytes: &[u8], format: Format) -> bool {
    match format {
        Format::Png => png_has_actl(bytes),
        Format::WebP => webp_animated(bytes),
        Format::Avif => avif_brands(bytes).is_some_and(|(major, compatible)| {
            major == *b"avis" || compatible.chunks_exact(4).any(|brand| brand == b"avis")
        }),
        Format::Jpeg => false,
    }
}

/// Width and height from the header, never zero.
pub fn dimensions(bytes: &[u8], format: Format) -> Option<(u32, u32)> {
    let (width, height) = match format {
        Format::Png => png_dimensions(bytes)?,
        Format::Jpeg => jpeg_dimensions(bytes)?,
        Format::WebP => webp_dimensions(bytes)?,
        Format::Avif => return None,
    };
    (width > 0 && height > 0).then_some((width, height))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16)
}

// PNG

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(bytes, 16)?, be32(bytes, 20)?))
}

/// An `acTL` chunk before the first `IDAT` marks APNG.
fn png_has_actl(bytes: &[u8]) -> bool {
    let mut at = PNG_SIGNATURE.len();
    while let (Some(length), Some(kind)) = (be32(bytes, at), bytes.get(at + 4..at + 8)) {
        match kind {
            b"acTL" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        at = match at.checked_add(12).and_then(|n| n.checked_add(length as usize)) {
            Some(next) => next,
            None => return false,
        };
    }
    false
}

// JPEG

/// Start-of-frame markers: C0 to CF except DHT (C4), JPG (C8) and DAC (CC).
fn is_sof(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    for segment in crate::metadata::parse_jpeg(bytes).segments {
        if is_sof(segment.marker) {
            let p = segment.payload;
            return Some((u32::from(be16(p, 3)?), u32::from(be16(p, 1)?)));
        }
    }
    None
}

// WebP

fn webp_animated(bytes: &[u8]) -> bool {
    match bytes.get(12..16) {
        Some(b"VP8X") => bytes.get(20).is_some_and(|flags| flags & 0x02 != 0),
        _ => false,
    }
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8X" => Some((le24(bytes, 24)? + 1, le24(bytes, 27)? + 1)),
        b"VP8 " => {
            if bytes.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let le16 = |at: usize| Some(u32::from(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?)));
            Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF))
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        _ => None,
    }
}

// AVIF

/// Major brand and the compatible-brands list of a leading `ftyp` box.
fn avif_brands(bytes: &[u8]) -> Option<([u8; 4], &[u8])> {
    if bytes.get(4..8)? != b"ftyp" {
        return None;
    }
    let size = be32(bytes, 0)? as usize;
    let end = size.max(16).min(bytes.len());
    let major: [u8; 4] = bytes.get(8..12)?.try_into().ok()?;
    // Bytes 12..16 are the minor version.
    Some((major, bytes.get(16..end)?))
}

fn is_avif(bytes: &[u8]) -> bool {
    avif_brands(bytes).is_some_and(|(major, compatible)| {
        [b"avif", b"avis"].iter().any(|brand| major == **brand)
            || compatible.chunks_exact(4).any(|b| b == b"avif" || b == b"avis")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0]);
        v.extend_from_slice(&[0; 4]); // CRC, not checked here
        v
    }

    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = (body.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v.extend_from_slice(&[0; 4]);
        v
    }

    fn jpeg_with_sof(width: u16, height: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        v.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]); // APP0, 2 payload bytes
        v.extend_from_slice(&[0xFF, 0xC2, 0x00, 0x0B, 8]); // progressive SOF2
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&[1, 1, 0x11, 0]);
        v
    }

    fn webp(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&((payload.len() + 12) as u32).to_le_bytes());
        v.extend_from_slice(b"WEBP");
        v.extend_from_slice(fourcc);
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn ftyp(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let size = 16 + 4 * compatible.len();
        let mut v = (size as u32).to_be_bytes().to_vec();
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(major);
        v.extend_from_slice(&[0; 4]);
        for brand in compatible {
            v.extend_from_slice(*brand);
        }
        v
    }

    #[test]
    fn detects_each_magic_number() {
        assert_eq!(detect(&png_header(1, 1)), Some(Format::Png));
        assert_eq!(detect(&jpeg_with_sof(1, 1)), Some(Format::Jpeg));
        assert_eq!(detect(&webp(b"VP8 ", &[0; 10])), Some(Format::WebP));
        assert_eq!(detect(&ftyp(b"avif", &[b"mif1"])), Some(Format::Avif));
        assert_eq!(detect(&ftyp(b"mif1", &[b"avif"])), Some(Format::Avif));
    }

    #[test]
    fn rejects_everything_else() {
        assert_eq!(detect(b""), None);
        assert_eq!(detect(b"GIF89a....."), None);
        assert_eq!(detect(b"\x89PNG"), None, "signature must be complete");
        assert_eq!(detect(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(detect(&ftyp(b"heic", &[b"mif1"])), None);
        assert_eq!(detect(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
    }

    #[test]
    fn reads_png_dimensions() {
        assert_eq!(dimensions(&png_header(640, 480), Format::Png), Some((640, 480)));
        assert_eq!(dimensions(&png_header(0, 480), Format::Png), None);
        assert_eq!(dimensions(PNG_SIGNATURE, Format::Png), None);
    }

    #[test]
    fn reads_jpeg_dimensions_past_other_segments() {
        assert_eq!(dimensions(&jpeg_with_sof(4000, 3000), Format::Jpeg), Some((4000, 3000)));
        assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF, 0xD9], Format::Jpeg), None);
    }

    #[test]
    fn reads_webp_dimensions() {
        let mut vp8x = vec![0x00, 0, 0, 0];
        vp8x.extend_from_slice(&[0x2B, 0x01, 0x00]); // width - 1 = 299
        vp8x.extend_from_slice(&[0xC7, 0x00, 0x00]); // height - 1 = 199
        assert_eq!(dimensions(&webp(b"VP8X", &vp8x), Format::WebP), Some((300, 200)));

        let mut vp8 = vec![0, 0, 0, 0x9D, 0x01, 0x2A];
        vp8.extend_from_slice(&320u16.to_le_bytes());
        vp8.extend_from_slice(&240u16.to_le_bytes());
        assert_eq!(dimensions(&webp(b"VP8 ", &vp8), Format::WebP), Some((320, 240)));

        let bits: u32 = 99 | (49 << 14);
        let mut vp8l = vec![0x2F];
        vp8l.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(dimensions(&webp(b"VP8L", &vp8l), Format::WebP), Some((100, 50)));
    }

    #[test]
    fn finds_apng_by_actl_before_idat() {
        let mut apng = png_header(2, 2);
        apng.extend(chunk(b"acTL", &[0; 8]));
        apng.extend(chunk(b"IDAT", &[0; 4]));
        assert!(is_animated(&apng, Format::Png));

        let mut still = png_header(2, 2);
        still.extend(chunk(b"IDAT", &[0; 4]));
        // An acTL after IDAT isn't valid and must not count.
        still.extend(chunk(b"acTL", &[0; 8]));
        assert!(!is_animated(&still, Format::Png));
    }

    #[test]
    fn finds_animated_webp_by_flag() {
        let animated = webp(b"VP8X", &[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let still = webp(b"VP8X", &[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert!(is_animated(&animated, Format::WebP));
        assert!(!is_animated(&still, Format::WebP));
        assert!(!is_animated(&webp(b"VP8 ", &[0; 10]), Format::WebP));
    }

    #[test]
    fn finds_avif_sequences_by_brand() {
        assert!(is_animated(&ftyp(b"avis", &[b"avif"]), Format::Avif));
        assert!(is_animated(&ftyp(b"avif", &[b"avis"]), Format::Avif));
        assert!(!is_animated(&ftyp(b"avif", &[b"mif1"]), Format::Avif));
    }

    #[test]
    fn jpeg_is_never_animated() {
        assert!(!is_animated(&jpeg_with_sof(1, 1), Format::Jpeg));
    }

    #[test]
    fn truncated_input_never_panics() {
        let full = png_header(3, 3);
        for n in 0..full.len() {
            let _ = inspect(&full[..n]);
        }
        let full = jpeg_with_sof(3, 3);
        for n in 0..full.len() {
            let _ = inspect(&full[..n]);
        }
        let full = ftyp(b"avif", &[b"mif1"]);
        for n in 0..full.len() {
            let _ = inspect(&full[..n]);
        }
    }
}

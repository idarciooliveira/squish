//! Metadata handling (ADR-0010): the color profile and orientation survive, the rest goes.
//!
//! This reads JPEG segments and PNG chunks directly. The formats are simple enough that
//! a parser here is shorter than a dependency, and it never touches pixel data.

use crate::options::MetaPolicy;

// JPEG

pub(crate) const SOI: u8 = 0xD8;
pub(crate) const EOI: u8 = 0xD9;
pub(crate) const SOS: u8 = 0xDA;
pub(crate) const APP0: u8 = 0xE0;
pub(crate) const APP1: u8 = 0xE1;
pub(crate) const APP2: u8 = 0xE2;
pub(crate) const APP14: u8 = 0xEE;
pub(crate) const COM: u8 = 0xFE;

const ICC_SIGNATURE: &[u8] = b"ICC_PROFILE\0";
const EXIF_SIGNATURE: &[u8] = b"Exif\0\0";

pub(crate) struct Segment<'a> {
    pub marker: u8,
    /// The bytes after the two length bytes. Empty for standalone markers.
    pub payload: &'a [u8],
    /// Offset of the `FF` byte, and the offset one past the last payload byte.
    pub start: usize,
    pub end: usize,
}

pub(crate) struct JpegHeader<'a> {
    /// Every segment before the first scan, in file order.
    pub segments: Vec<Segment<'a>>,
    /// Offset of the first `FF DA`, where entropy-coded data begins.
    pub scan_start: Option<usize>,
}

/// Splits a JPEG into the segments before its first scan. Stops quietly at damage.
pub(crate) fn parse_jpeg(bytes: &[u8]) -> JpegHeader<'_> {
    let mut segments = Vec::new();
    let mut scan_start = None;
    if bytes.get(..2) != Some(&[0xFF, SOI]) {
        return JpegHeader { segments, scan_start };
    }
    let mut at = 2;
    while at + 1 < bytes.len() {
        if bytes[at] != 0xFF {
            break;
        }
        let start = at;
        // Any number of 0xFF fill bytes may precede the marker code.
        while bytes.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        let Some(&marker) = bytes.get(at + 1) else { break };
        at += 2;
        match marker {
            EOI => break,
            SOS => {
                scan_start = Some(start);
                break;
            }
            // Standalone markers carry no length: TEM, RSTn, SOI.
            0x01 | 0xD0..=0xD8 => {
                segments.push(Segment { marker, payload: &[], start, end: at });
            }
            0x00 => break,
            _ => {
                let Some(len) = bytes.get(at..at + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]]))) else {
                    break;
                };
                if len < 2 || at + len > bytes.len() {
                    break;
                }
                segments.push(Segment { marker, payload: &bytes[at + 2..at + len], start, end: at + len });
                at += len;
            }
        }
    }
    JpegHeader { segments, scan_start }
}

/// The ICC profile, reassembled from its APP2 chunks.
///
/// The spec numbers chunks from 1. Some writers, including the `mozjpeg` crate's
/// `write_icc_profile`, start at 0, so both are accepted.
pub(crate) fn jpeg_icc(segments: &[Segment<'_>]) -> Option<Vec<u8>> {
    let mut parts: Vec<(u8, u8, &[u8])> = segments
        .iter()
        .filter(|s| {
            s.marker == APP2 && s.payload.starts_with(ICC_SIGNATURE) && s.payload.len() > ICC_SIGNATURE.len() + 2
        })
        .map(|s| {
            let p = &s.payload[ICC_SIGNATURE.len()..];
            (p[0], p[1], &p[2..])
        })
        .collect();
    let count = parts.first()?.1;
    if count == 0 || parts.len() != usize::from(count) || parts.iter().any(|p| p.1 != count) {
        return None;
    }
    parts.sort_by_key(|p| p.0);
    let first = usize::from(parts[0].0);
    if first > 1 || parts.iter().enumerate().any(|(i, p)| usize::from(p.0) != first + i) {
        return None;
    }
    Some(parts.iter().flat_map(|p| p.2.iter().copied()).collect())
}

/// APP2 payloads that carry `profile`, split at the 64 KB segment limit and numbered from 1.
pub(crate) fn icc_payloads(profile: &[u8]) -> Vec<Vec<u8>> {
    // 65535 segment length, minus 2 for the length field, 12 for the signature and 2 for the sequence.
    const MAX_DATA: usize = 65535 - 2 - 12 - 2;
    let chunks: Vec<&[u8]> = profile.chunks(MAX_DATA).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut p = ICC_SIGNATURE.to_vec();
            p.extend_from_slice(&[(i + 1) as u8, chunks.len() as u8]);
            p.extend_from_slice(chunk);
            p
        })
        .collect()
}

/// Orientation (1 to 8) from the first Exif APP1 segment.
pub(crate) fn jpeg_orientation(segments: &[Segment<'_>]) -> Option<u8> {
    segments
        .iter()
        .filter(|s| s.marker == APP1)
        .find_map(|s| s.payload.strip_prefix(EXIF_SIGNATURE))
        .and_then(tiff_orientation)
}

/// Reads tag 0x0112 from IFD0 of a TIFF block.
fn tiff_orientation(tiff: &[u8]) -> Option<u8> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    if u16_at(2)? != 42 {
        return None;
    }
    let ifd = u32_at(4)? as usize;
    let entries = usize::from(u16_at(ifd)?);
    for i in 0..entries {
        let entry = ifd.checked_add(2 + i * 12)?;
        if u16_at(entry)? == 0x0112 {
            // Type 3 (SHORT), count 1, the value sits in the first two bytes of the value field.
            let value = u16_at(entry + 8)?;
            return (u16_at(entry + 2)? == 3 && (1..=8).contains(&value)).then_some(value as u8);
        }
    }
    None
}

/// An Exif APP1 payload that holds only the orientation tag. 32 bytes.
pub(crate) fn minimal_exif(orientation: u8) -> Vec<u8> {
    let mut v = Vec::with_capacity(32);
    v.extend_from_slice(EXIF_SIGNATURE);
    v.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08"); // big-endian TIFF, IFD0 at offset 8
    v.extend_from_slice(&1u16.to_be_bytes()); // one entry
    v.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    v.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    v.extend_from_slice(&1u32.to_be_bytes()); // count
    v.extend_from_slice(&[0, orientation, 0, 0]); // value, left-justified
    v.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
    v
}

/// What to put into a re-encoded JPEG, decided by the policy.
#[derive(Default)]
pub(crate) struct JpegMeta {
    pub icc: Option<Vec<u8>>,
    /// APPn or COM segments to write as they are: `(marker, payload)`.
    pub segments: Vec<(u8, Vec<u8>)>,
}

pub(crate) fn jpeg_meta(bytes: &[u8], policy: MetaPolicy) -> JpegMeta {
    let header = parse_jpeg(bytes);
    let mut meta = JpegMeta::default();
    if policy == MetaPolicy::StripAll {
        return meta;
    }
    meta.icc = jpeg_icc(&header.segments);
    match policy {
        MetaPolicy::StripKeepIcc => {
            // Orientation 1 means "as stored", the same as no tag.
            if let Some(o) = jpeg_orientation(&header.segments).filter(|&o| o != 1) {
                meta.segments.push((APP1, minimal_exif(o)));
            }
        }
        MetaPolicy::KeepAll => {
            for s in &header.segments {
                // JFIF is rewritten by the encoder. Adobe's APP14 describes the source
                // color transform, which doesn't apply to our output. APP2 holds the ICC
                // profile (written above) or an MPF index whose offsets would be stale.
                let keep = match s.marker {
                    APP0 | APP2 | APP14 => false,
                    APP1..=0xEF | COM => true,
                    _ => false,
                };
                if keep {
                    meta.segments.push((s.marker, s.payload.to_vec()));
                }
            }
        }
        MetaPolicy::StripAll => unreachable!(),
    }
    meta
}

/// Applies the policy to a JPEG without touching its entropy-coded data.
pub(crate) fn jpeg_strip(bytes: &[u8], policy: MetaPolicy) -> Vec<u8> {
    let header = parse_jpeg(bytes);
    let Some(scan_start) = header.scan_start else { return bytes.to_vec() };
    let orientation = jpeg_orientation(&header.segments).filter(|&o| o != 1);
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&[0xFF, SOI]);
    let mut wrote_exif = false;
    for s in &header.segments {
        let is_icc = s.marker == APP2 && s.payload.starts_with(ICC_SIGNATURE);
        let keep = match s.marker {
            // Everything that isn't metadata, plus JFIF and Adobe, which decoders need.
            m if !(APP0..=0xEF).contains(&m) && m != COM => true,
            APP0 | APP14 => true,
            _ => match policy {
                MetaPolicy::KeepAll => true,
                MetaPolicy::StripAll => false,
                MetaPolicy::StripKeepIcc => is_icc,
            },
        };
        if keep {
            out.extend_from_slice(&bytes[s.start..s.end]);
        }
        // Put the minimal orientation block where the first dropped Exif segment was.
        if policy == MetaPolicy::StripKeepIcc
            && s.marker == APP1
            && !wrote_exif
            && !keep
            && let Some(o) = orientation
        {
            let payload = minimal_exif(o);
            out.extend_from_slice(&[0xFF, APP1]);
            out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
            out.extend_from_slice(&payload);
            wrote_exif = true;
        }
    }
    // Data after the primary image's EOI (MPF secondary images, vendor trailers) can carry
    // its own Exif, GPS included. Only `KeepAll` keeps it.
    let end = match policy {
        MetaPolicy::KeepAll => None,
        _ => primary_image_end(bytes, scan_start),
    };
    out.extend_from_slice(&bytes[scan_start..end.unwrap_or(bytes.len())]);
    out
}

/// Offset just past the EOI of the image whose first scan starts at `at`. Skips
/// inter-scan segments by their lengths, so table bytes are never mistaken for EOI.
/// `None` if the data ends or breaks first.
fn primary_image_end(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        while *bytes.get(at + 1)? == 0xFF {
            at += 1;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        match marker {
            EOI => return Some(at),
            0x01 | 0xD0..=0xD7 => {}
            0x00 => return None,
            _ => {
                let len = usize::from(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
                if len < 2 {
                    return None;
                }
                at += len;
                if marker == SOS {
                    // Entropy-coded data: 0xFF is stuffed as FF 00, and RSTn may appear inline.
                    loop {
                        if *bytes.get(at)? != 0xFF {
                            at += 1;
                            continue;
                        }
                        match *bytes.get(at + 1)? {
                            0x00 | 0xD0..=0xD7 => at += 2,
                            _ => break,
                        }
                    }
                }
            }
        }
    }
}

// PNG

pub(crate) struct PngChunk<'a> {
    pub kind: [u8; 4],
    /// Length, type, data and CRC, exactly as stored.
    pub raw: &'a [u8],
}

/// Walks chunks up to `IEND` or the first damaged one.
pub(crate) fn png_chunks(bytes: &[u8]) -> Vec<PngChunk<'_>> {
    let mut chunks = Vec::new();
    let mut at = 8;
    while let Some(head) = bytes.get(at..at + 8) {
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = [head[4], head[5], head[6], head[7]];
        let Some(end) = at.checked_add(12).and_then(|n| n.checked_add(length)) else { break };
        let Some(raw) = bytes.get(at..end) else { break };
        chunks.push(PngChunk { kind, raw });
        if &kind == b"IEND" {
            break;
        }
        at = end;
    }
    chunks
}

/// PNG chunks that say how to interpret the pixel values. `strip-keep-icc` keeps them: a
/// PNG without `iCCP` may still carry its color space in `gAMA` and `cHRM`, or in `cICP`,
/// which `mDCV` and `cLLI` complete for HDR.
pub(crate) const PNG_COLOR_CHUNKS: [[u8; 4]; 7] =
    [*b"iCCP", *b"sRGB", *b"gAMA", *b"cHRM", *b"cICP", *b"mDCV", *b"cLLI"];

/// Color chunk types that `strip-keep-icc` keeps from `original`. A file shouldn't carry
/// both `iCCP` and `sRGB`, and PNG 3rd edition ranks `iCCP` higher, so `sRGB` goes then.
/// Leaving `sRGB` out of oxipng's keep set also stops it from rewriting the profile.
pub(crate) fn png_color_kinds(original: &[u8]) -> Vec<[u8; 4]> {
    let present: Vec<[u8; 4]> = png_chunks(original).iter().map(|c| c.kind).collect();
    let has_icc = present.contains(b"iCCP");
    PNG_COLOR_CHUNKS.into_iter().filter(|k| present.contains(k) && !(has_icc && k == b"sRGB")).collect()
}

/// True when `cICP` tags the PNG with an HDR transfer function, PQ (16) or HLG (18).
pub(crate) fn png_is_hdr(original: &[u8]) -> bool {
    png_chunks(original).iter().any(|c| &c.kind == b"cICP" && matches!(c.raw.get(9), Some(16 | 18)))
}

/// Ancillary chunks of the original that the policy keeps, as raw bytes.
pub(crate) fn png_chunks_to_keep(original: &[u8], policy: MetaPolicy) -> Vec<&[u8]> {
    let chunks = png_chunks(original);
    match policy {
        MetaPolicy::StripAll => vec![],
        MetaPolicy::StripKeepIcc => {
            let kinds = png_color_kinds(original);
            chunks.iter().filter(|c| kinds.contains(&c.kind)).map(|c| c.raw).collect()
        }
        MetaPolicy::KeepAll => chunks
            .iter()
            .filter(|c| {
                let ancillary = c.kind[0] & 0x20 != 0;
                // These describe the original's pixel data or its palette. Our output has its own.
                let describes_pixels =
                    matches!(&c.kind, b"tRNS" | b"bKGD" | b"hIST" | b"sBIT" | b"acTL" | b"fcTL" | b"fdAT");
                ancillary && !describes_pixels
            })
            .map(|c| c.raw)
            .collect(),
    }
}

/// Inserts raw chunks right after IHDR, which is valid for every chunk `png_chunks_to_keep` returns.
pub(crate) fn png_insert_after_ihdr(png: Vec<u8>, extra: &[&[u8]]) -> Vec<u8> {
    const AFTER_IHDR: usize = 8 + 25;
    if extra.is_empty() || png.len() < AFTER_IHDR {
        return png;
    }
    let mut out = Vec::with_capacity(png.len() + extra.iter().map(|c| c.len()).sum::<usize>());
    out.extend_from_slice(&png[..AFTER_IHDR]);
    for chunk in extra {
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&png[AFTER_IHDR..]);
    out
}

// WebP

// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) struct WebpChunk<'a> {
    pub kind: [u8; 4],
    /// The payload, without the 8-byte header or the pad byte.
    pub payload: &'a [u8],
}

/// Walks the top-level chunks after `WEBP`, stopping at the first damaged one.
// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) fn webp_chunks(bytes: &[u8]) -> Vec<WebpChunk<'_>> {
    let mut chunks = Vec::new();
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WEBP") {
        return chunks;
    }
    let mut at = 12;
    while let Some(head) = bytes.get(at..at + 8) {
        let kind = [head[0], head[1], head[2], head[3]];
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let Some(end) = at.checked_add(8).and_then(|n| n.checked_add(size)) else { break };
        let Some(payload) = bytes.get(at + 8..end) else { break };
        chunks.push(WebpChunk { kind, payload });
        // Odd payloads end with a zero pad byte that the size doesn't count.
        at = end + (size & 1);
    }
    chunks
}

/// True when the image data is a `VP8L` bitstream. A lossy file with alpha has `ALPH` + `VP8 `.
// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) fn webp_is_lossless(bytes: &[u8]) -> bool {
    webp_chunks(bytes).iter().any(|c| &c.kind == b"VP8L")
}

/// Orientation 1 to 8 from a WebP `EXIF` payload, with or without the `Exif\0\0` prefix.
// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) fn webp_orientation(exif: &[u8]) -> Option<u8> {
    tiff_orientation(exif.strip_prefix(EXIF_SIGNATURE).unwrap_or(exif))
}

/// The chunks around the image data that go into the output.
#[derive(Debug, Default, PartialEq)]
// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) struct WebpMeta {
    pub icc: Option<Vec<u8>>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    /// Chunks squish doesn't know, in their original order. Only `keep-all` fills this.
    pub unknown: Vec<([u8; 4], Vec<u8>)>,
}

/// Image-data chunks that never carry metadata: the bitstreams, their alpha sidecar,
/// the container header they hang under, and animation frames, which squish rejects.
// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
const WEBP_IMAGE_CHUNKS: [[u8; 4]; 6] = [*b"VP8X", *b"VP8 ", *b"VP8L", *b"ALPH", *b"ANIM", *b"ANMF"];

// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) fn webp_meta(bytes: &[u8], policy: MetaPolicy) -> WebpMeta {
    let chunks = webp_chunks(bytes);
    let mut meta = WebpMeta::default();
    if policy == MetaPolicy::StripAll {
        return meta;
    }
    meta.icc = chunks.iter().find(|c| &c.kind == b"ICCP").map(|c| c.payload.to_vec());
    match policy {
        MetaPolicy::StripKeepIcc => {
            // Orientation 1 means "as stored", the same as no tag.
            if let Some(o) =
                chunks.iter().find(|c| &c.kind == b"EXIF").and_then(|c| webp_orientation(c.payload)).filter(|&o| o != 1)
            {
                // `minimal_exif` returns a JPEG APP1 payload starting with `Exif\0\0`;
                // the WebP chunk holds the bare TIFF data.
                meta.exif = Some(minimal_exif(o)[EXIF_SIGNATURE.len()..].to_vec());
            }
        }
        MetaPolicy::KeepAll => {
            meta.exif = chunks.iter().find(|c| &c.kind == b"EXIF").map(|c| c.payload.to_vec());
            meta.xmp = chunks.iter().find(|c| &c.kind == b"XMP ").map(|c| c.payload.to_vec());
            for c in &chunks {
                if !WEBP_IMAGE_CHUNKS.contains(&c.kind)
                    && &c.kind != b"ICCP"
                    && &c.kind != b"EXIF"
                    && &c.kind != b"XMP "
                {
                    meta.unknown.push((c.kind, c.payload.to_vec()));
                }
            }
        }
        MetaPolicy::StripAll => unreachable!(),
    }
    meta
}

// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
fn webp_push(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
}

/// Builds a WebP file from the image chunks of `encoded` (a complete file from libwebp,
/// or the original) and `meta`. `width` and `height` are the canvas size.
// Used by codec/webp.rs from the next commit.
#[allow(dead_code)]
pub(crate) fn webp_assemble(encoded: &[u8], meta: &WebpMeta, width: u32, height: u32) -> Vec<u8> {
    let chunks = webp_chunks(encoded);
    let image: Vec<&WebpChunk<'_>> =
        chunks.iter().filter(|c| &c.kind == b"ALPH" || &c.kind == b"VP8 " || &c.kind == b"VP8L").collect();
    let has_alpha = image.iter().any(|c| &c.kind == b"ALPH");
    let meta_empty = meta.icc.is_none() && meta.exif.is_none() && meta.xmp.is_none() && meta.unknown.is_empty();

    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"WEBP");
    if meta_empty && !has_alpha {
        for c in &image {
            webp_push(&mut out, &c.kind, c.payload);
        }
    } else {
        // The alpha flag survives even when the `ALPH` chunk comes from the encoder:
        // a `VP8X` header may already carry it, and a `VP8L` bitstream has it in its
        // header (bit 28 of the u32 after the `0x2F` signature byte) without an `ALPH` chunk.
        let alpha = has_alpha
            || chunks
                .iter()
                .find(|c| &c.kind == b"VP8X")
                .is_some_and(|c| c.payload.first().is_some_and(|f| f & 0x10 != 0))
            || image.iter().any(|c| {
                &c.kind == b"VP8L"
                    && c.payload.first() == Some(&0x2F)
                    && matches!(c.payload.get(1..5), Some(h) if u32::from_le_bytes([h[0], h[1], h[2], h[3]]) >> 28 & 1 == 1)
            });
        let mut flags = 0u8;
        if meta.icc.is_some() {
            flags |= 0x20;
        }
        if alpha {
            flags |= 0x10;
        }
        if meta.exif.is_some() {
            flags |= 0x08;
        }
        if meta.xmp.is_some() {
            flags |= 0x04;
        }
        let mut vp8x = [0u8; 10];
        vp8x[0] = flags;
        vp8x[4..7].copy_from_slice(&width.saturating_sub(1).to_le_bytes()[..3]);
        vp8x[7..10].copy_from_slice(&height.saturating_sub(1).to_le_bytes()[..3]);
        webp_push(&mut out, b"VP8X", &vp8x);
        if let Some(icc) = &meta.icc {
            webp_push(&mut out, b"ICCP", icc);
        }
        // `ALPH` before `VP8 `: lossy with alpha decodes only in that order.
        for c in image.iter().filter(|c| &c.kind == b"ALPH") {
            webp_push(&mut out, &c.kind, c.payload);
        }
        for c in image.iter().filter(|c| &c.kind != b"ALPH") {
            webp_push(&mut out, &c.kind, c.payload);
        }
        for (kind, payload) in &meta.unknown {
            webp_push(&mut out, kind, payload);
        }
        if let Some(exif) = &meta.exif {
            webp_push(&mut out, b"EXIF", exif);
        }
        if let Some(xmp) = &meta.xmp {
            webp_push(&mut out, b"XMP ", xmp);
        }
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn jpeg(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0xFF, SOI];
        for p in parts {
            v.extend_from_slice(p);
        }
        // SOS header, two bytes of entropy data, EOI.
        v.extend_from_slice(&app(SOS, &[1, 1, 0, 0, 0x3F, 0]));
        v.extend_from_slice(&[0x12, 0x34, 0xFF, EOI]);
        v
    }

    fn icc(seq: u8, count: u8, data: &[u8]) -> Vec<u8> {
        let mut p = ICC_SIGNATURE.to_vec();
        p.extend_from_slice(&[seq, count]);
        p.extend_from_slice(data);
        app(APP2, &p)
    }

    #[test]
    fn minimal_exif_reads_back() {
        for o in 1..=8 {
            let exif = minimal_exif(o);
            assert_eq!(exif.len(), 32);
            assert_eq!(tiff_orientation(exif.strip_prefix(EXIF_SIGNATURE).unwrap()), Some(o));
        }
    }

    #[test]
    fn reads_orientation_in_both_byte_orders() {
        // Little-endian block with Make (tag 0x010F) before Orientation, to prove it scans.
        let mut tiff = b"II\x2A\x00\x08\x00\x00\x00".to_vec();
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&[0x0F, 0x01, 2, 0, 1, 0, 0, 0, b'X', 0, 0, 0]);
        tiff.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(tiff_orientation(&tiff), Some(6));
        assert_eq!(tiff_orientation(b"MM\x00\x2A\x00\x00\x00\x08\x00\x00"), None);
        assert_eq!(tiff_orientation(b"XX"), None);
    }

    #[test]
    fn rejects_out_of_range_orientation() {
        let mut exif = minimal_exif(1);
        exif[6 + 8 + 2 + 9] = 9;
        assert_eq!(tiff_orientation(exif.strip_prefix(EXIF_SIGNATURE).unwrap()), None);
    }

    #[test]
    fn reassembles_split_icc_profile_in_sequence_order() {
        let file = jpeg(&[icc(2, 2, b"world"), icc(1, 2, b"hello ")]);
        let header = parse_jpeg(&file);
        assert_eq!(jpeg_icc(&header.segments), Some(b"hello world".to_vec()));
    }

    #[test]
    fn zero_based_icc_numbering_is_accepted() {
        let file = jpeg(&[icc(0, 2, b"zero "), icc(1, 2, b"based")]);
        assert_eq!(jpeg_icc(&parse_jpeg(&file).segments), Some(b"zero based".to_vec()));
    }

    #[test]
    fn icc_payloads_round_trip_across_segments() {
        let profile: Vec<u8> = (0..150_000u32).map(|i| (i % 253) as u8).collect();
        let payloads = icc_payloads(&profile);
        assert_eq!(payloads.len(), 3);
        assert!(payloads.iter().all(|p| p.len() + 2 <= 65535));
        let file = jpeg(&payloads.iter().map(|p| app(APP2, p)).collect::<Vec<_>>());
        assert_eq!(jpeg_icc(&parse_jpeg(&file).segments), Some(profile));
        assert!(payloads.iter().enumerate().all(|(i, p)| p[12] == i as u8 + 1 && p[13] == 3));
    }

    #[test]
    fn incomplete_icc_profile_is_dropped() {
        let file = jpeg(&[icc(1, 2, b"half")]);
        assert_eq!(jpeg_icc(&parse_jpeg(&file).segments), None);
    }

    #[test]
    fn keep_icc_policy_swaps_exif_for_minimal_orientation() {
        let mut exif = EXIF_SIGNATURE.to_vec();
        exif.extend_from_slice(b"MM\x00\x2A\x00\x00\x00\x08\x00\x02");
        exif.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 8, 0, 0]); // Orientation 8
        exif.extend_from_slice(&[0x88, 0x25, 0, 4, 0, 0, 0, 1, 0, 0, 0, 0]); // GPS pointer
        exif.extend_from_slice(&[0; 4]);
        let file = jpeg(&[
            app(APP0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"),
            app(APP1, &exif),
            icc(1, 1, b"profile"),
            app(0xED, b"Photoshop 3.0\0..."),
            app(COM, b"a comment"),
        ]);

        let stripped = jpeg_strip(&file, MetaPolicy::StripKeepIcc);
        let header = parse_jpeg(&stripped);
        assert_eq!(jpeg_icc(&header.segments), Some(b"profile".to_vec()));
        assert_eq!(jpeg_orientation(&header.segments), Some(8));
        assert!(header.segments.iter().all(|s| s.marker != COM && s.marker != 0xED));
        assert!(stripped.len() < file.len());
        assert!(stripped.ends_with(&[0x12, 0x34, 0xFF, EOI]), "scan data must be untouched");
        assert!(!stripped.windows(2).any(|w| w == [0x88, 0x25]), "GPS entry must be gone");
    }

    #[test]
    fn strip_all_keeps_only_what_decoders_need() {
        let file = jpeg(&[
            app(APP0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"),
            app(APP1, EXIF_SIGNATURE),
            icc(1, 1, b"profile"),
            app(APP14, b"Adobe\0\0\0\0\0\0\0"),
            app(0xDB, &[0; 65]), // DQT
        ]);
        let stripped = jpeg_strip(&file, MetaPolicy::StripAll);
        let header = parse_jpeg(&stripped);
        let markers: Vec<u8> = header.segments.iter().map(|s| s.marker).collect();
        assert_eq!(markers, [APP0, APP14, 0xDB]);
    }

    #[test]
    fn keep_all_policy_returns_file_unchanged() {
        let file = jpeg(&[app(APP0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"), app(APP1, b"Exif\0\0xx"), app(COM, b"hi")]);
        assert_eq!(jpeg_strip(&file, MetaPolicy::KeepAll), file);
    }

    #[test]
    fn strip_drops_trailing_images() {
        let primary = jpeg(&[app(APP1, b"Exif\0\0gps"), app(0xDB, &[0; 65])]);
        // A second scan whose DHT payload holds FF D9, as a progressive file might.
        let mut progressive = primary[..primary.len() - 2].to_vec();
        progressive.extend_from_slice(&app(0xC4, &[0x10, 0xFF, EOI, 0x00]));
        progressive.extend_from_slice(&app(SOS, &[1, 1, 0, 0, 0x3F, 0]));
        progressive.extend_from_slice(&[0xFF, 0x00, 0xFF, 0xD0, 0x56, 0xFF, EOI]);
        let secondary = jpeg(&[app(APP1, b"Exif\0\0gps")]);
        for image in [primary, progressive] {
            let mut file = image.clone();
            file.extend_from_slice(&secondary);
            for policy in [MetaPolicy::StripAll, MetaPolicy::StripKeepIcc] {
                let stripped = jpeg_strip(&file, policy);
                let scan = parse_jpeg(&image).scan_start.unwrap();
                assert!(stripped.ends_with(&image[scan..]), "{policy:?}");
                assert!(!stripped.windows(4).any(|w| w == b"Exif"), "{policy:?}");
            }
            assert_eq!(jpeg_strip(&file, MetaPolicy::KeepAll), file);
        }
    }

    #[test]
    fn re_encode_meta_follows_policy() {
        let file = jpeg(&[app(APP1, b"Exif\0\0nope"), icc(1, 1, b"p"), app(COM, b"c"), app(0xED, b"iptc")]);
        let all = jpeg_meta(&file, MetaPolicy::KeepAll);
        assert_eq!(all.icc.as_deref(), Some(&b"p"[..]));
        let markers: Vec<u8> = all.segments.iter().map(|s| s.0).collect();
        assert_eq!(markers, [APP1, COM, 0xED]);
        assert!(jpeg_meta(&file, MetaPolicy::StripAll).icc.is_none());
        let keep = jpeg_meta(&file, MetaPolicy::StripKeepIcc);
        assert!(keep.icc.is_some() && keep.segments.is_empty(), "no orientation tag, so no Exif block");
    }

    #[test]
    fn damaged_jpeg_never_panics() {
        let file = jpeg(&[app(APP1, b"Exif\0\0MM\x00\x2A\xFF\xFF\xFF\xFF"), icc(1, 1, b"p")]);
        for n in 0..file.len() {
            let cut = &file[..n];
            let _ = jpeg_strip(cut, MetaPolicy::StripKeepIcc);
            let _ = jpeg_meta(cut, MetaPolicy::KeepAll);
        }
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(data);
        v.extend_from_slice(&[0xAA; 4]);
        v
    }

    fn png(chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend(png_chunk(b"IHDR", &[0; 13]));
        for c in chunks {
            v.extend_from_slice(c);
        }
        v.extend(png_chunk(b"IEND", &[]));
        v
    }

    #[test]
    fn png_policy_selects_chunks() {
        let file = png(&[
            png_chunk(b"iCCP", b"icc\0\0zz"),
            png_chunk(b"sRGB", &[0]),
            png_chunk(b"gAMA", &[0, 0, 0xB1, 0x8F]),
            png_chunk(b"tEXt", b"Comment\0hi"),
            png_chunk(b"tRNS", &[0]),
            png_chunk(b"pHYs", &[0; 9]),
            png_chunk(b"IDAT", &[1, 2, 3]),
        ]);
        let kinds = |policy| -> Vec<[u8; 4]> {
            png_chunks_to_keep(&file, policy).iter().map(|raw| [raw[4], raw[5], raw[6], raw[7]]).collect()
        };
        assert!(kinds(MetaPolicy::StripAll).is_empty());
        assert_eq!(kinds(MetaPolicy::StripKeepIcc), [*b"iCCP", *b"gAMA"]);
        assert_eq!(kinds(MetaPolicy::KeepAll), [*b"iCCP", *b"sRGB", *b"gAMA", *b"tEXt", *b"pHYs"]);
    }

    #[test]
    fn png_srgb_is_kept_only_without_an_icc_profile() {
        let srgb = png(&[png_chunk(b"sRGB", &[0]), png_chunk(b"gAMA", &[0, 0, 0xB1, 0x8F])]);
        assert_eq!(png_color_kinds(&srgb), [*b"sRGB", *b"gAMA"]);
        let both = png(&[png_chunk(b"iCCP", b"icc\0\0zz"), png_chunk(b"sRGB", &[0])]);
        assert_eq!(png_color_kinds(&both), [*b"iCCP"]);
    }

    #[test]
    fn png_hdr_is_read_from_the_cicp_transfer_function() {
        let cicp = |transfer| png(&[png_chunk(b"cICP", &[9, transfer, 0, 1])]);
        assert!(png_is_hdr(&cicp(16)) && png_is_hdr(&cicp(18)));
        assert!(!png_is_hdr(&cicp(13)) && !png_is_hdr(&png(&[])));
    }

    #[test]
    fn png_insert_lands_between_ihdr_and_the_rest() {
        let base = png(&[png_chunk(b"IDAT", &[9])]);
        let extra = png_chunk(b"iCCP", b"x");
        let merged = png_insert_after_ihdr(base, &[&extra]);
        let kinds: Vec<[u8; 4]> = png_chunks(&merged).iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [*b"IHDR", *b"iCCP", *b"IDAT", *b"IEND"]);
    }

    fn webp(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut v = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
        for (kind, payload) in chunks {
            v.extend_from_slice(*kind);
            v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            v.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                v.push(0);
            }
        }
        let size = (v.len() - 8) as u32;
        v[4..8].copy_from_slice(&size.to_le_bytes());
        v
    }

    #[test]
    fn webp_chunks_reads_odd_chunks_and_stops_on_damage() {
        let file = webp(&[(b"VP8 ", &[1, 2, 3]), (b"ICCP", &[9; 4])]);
        let chunks = webp_chunks(&file);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].kind, *b"VP8 ");
        assert_eq!(chunks[0].payload, [1, 2, 3]);
        // The odd payload's pad byte is skipped, so the next chunk still parses.
        assert_eq!(chunks[1].payload, [9; 4]);
        let cut = &file[..file.len() - 3];
        assert_eq!(webp_chunks(cut).len(), 1);
        assert!(webp_chunks(b"not a webp file").is_empty());
    }

    #[test]
    fn webp_lossless_is_vp8l_only() {
        assert!(webp_is_lossless(&webp(&[(b"VP8L", &[0x2F, 1, 2, 3])])));
        assert!(!webp_is_lossless(&webp(&[(b"VP8 ", &[7; 8])])));
        assert!(!webp_is_lossless(&webp(&[(b"ALPH", &[1]), (b"VP8 ", &[7; 8])])));
    }

    #[test]
    fn webp_orientation_reads_both_byte_orders_and_the_prefix() {
        let bare = &minimal_exif(6)[EXIF_SIGNATURE.len()..];
        assert_eq!(webp_orientation(bare), Some(6));
        let mut prefixed = EXIF_SIGNATURE.to_vec();
        prefixed.extend_from_slice(bare);
        assert_eq!(webp_orientation(&prefixed), Some(6));
        // Little-endian block with orientation 3.
        let mut tiff = b"II\x2A\x00\x08\x00\x00\x00".to_vec();
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(webp_orientation(&tiff), Some(3));
        assert_eq!(webp_orientation(b"garbage"), None);
    }

    #[test]
    fn webp_meta_follows_the_policy() {
        let tiff = minimal_exif(6)[EXIF_SIGNATURE.len()..].to_vec();
        let file = webp(&[
            (b"VP8X", &[0x38; 10]),
            (b"ICCP", b"profile"),
            (b"VP8 ", &[7; 8]),
            (b"ZZZZ", b"one"),
            (b"XMP ", b"<x/>"),
            (b"ABCD", b"two"),
            (b"EXIF", &tiff),
        ]);
        let keep = webp_meta(&file, MetaPolicy::StripKeepIcc);
        assert_eq!(keep.icc.as_deref(), Some(&b"profile"[..]));
        assert_eq!(keep.xmp, None);
        assert!(keep.unknown.is_empty());
        let kept_exif = keep.exif.expect("orientation 6 survives");
        assert_eq!(kept_exif, minimal_exif(6)[EXIF_SIGNATURE.len()..]);
        assert_eq!(webp_orientation(&kept_exif), Some(6));

        assert_eq!(webp_meta(&file, MetaPolicy::StripAll), WebpMeta::default());

        let all = webp_meta(&file, MetaPolicy::KeepAll);
        assert_eq!(all.icc.as_deref(), Some(&b"profile"[..]));
        assert_eq!(all.exif.as_deref(), Some(&tiff[..]));
        assert_eq!(all.xmp.as_deref(), Some(&b"<x/>"[..]));
        assert_eq!(all.unknown, [(*b"ZZZZ", b"one".to_vec()), (*b"ABCD", b"two".to_vec())]);

        // Orientation 1 is "as stored", the same as no tag.
        let upright_tiff = minimal_exif(1)[EXIF_SIGNATURE.len()..].to_vec();
        let upright = webp(&[(b"VP8 ", &[7; 8]), (b"EXIF", &upright_tiff)]);
        assert!(webp_meta(&upright, MetaPolicy::StripKeepIcc).exif.is_none());
    }

    #[test]
    fn webp_assemble_writes_simple_and_extended_files() {
        // Empty meta and no alpha: a plain bitstream file.
        let simple = webp_assemble(&webp(&[(b"VP8 ", &[7; 8])]), &WebpMeta::default(), 16, 16);
        let kinds: Vec<[u8; 4]> = webp_chunks(&simple).iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [*b"VP8 "]);
        assert_eq!(u32::from_le_bytes(simple[4..8].try_into().unwrap()) as usize, simple.len() - 8);

        // With metadata: VP8X flags, canvas size, chunk order, RIFF size.
        let meta = WebpMeta {
            icc: Some(b"profile".to_vec()),
            exif: Some(minimal_exif(6)[EXIF_SIGNATURE.len()..].to_vec()),
            xmp: Some(b"<x/>".to_vec()),
            unknown: vec![(*b"ZZZZ", b"one".to_vec())],
        };
        let out = webp_assemble(&webp(&[(b"VP8 ", &[7; 8])]), &meta, 192, 128);
        let chunks = webp_chunks(&out);
        let kinds: Vec<[u8; 4]> = chunks.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [*b"VP8X", *b"ICCP", *b"VP8 ", *b"ZZZZ", *b"EXIF", *b"XMP "]);
        assert_eq!(chunks[0].payload.len(), 10);
        assert_eq!(chunks[0].payload[0], 0x20 | 0x08 | 0x04, "ICC, EXIF and XMP flags, no alpha");
        assert_eq!(&chunks[0].payload[4..7], &191u32.to_le_bytes()[..3]);
        assert_eq!(&chunks[0].payload[7..10], &127u32.to_le_bytes()[..3]);
        assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()) as usize, out.len() - 8);

        // An ALPH chunk forces the extended container with the alpha flag, ALPH first.
        let alpha = webp_assemble(&webp(&[(b"VP8 ", &[7; 8]), (b"ALPH", &[1])]), &WebpMeta::default(), 16, 16);
        let chunks = webp_chunks(&alpha);
        let kinds: Vec<[u8; 4]> = chunks.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [*b"VP8X", *b"ALPH", *b"VP8 "]);
        assert_eq!(chunks[0].payload[0] & 0x10, 0x10);
    }

    #[test]
    fn damaged_webp_never_panics() {
        let tiff = minimal_exif(6)[EXIF_SIGNATURE.len()..].to_vec();
        let file = webp(&[
            (b"VP8X", &[0x3C; 10]),
            (b"ICCP", b"profile"),
            (b"ALPH", &[1, 2, 3]),
            (b"VP8 ", &[7; 8]),
            (b"ZZZZ", b"one"),
            (b"EXIF", &tiff),
            (b"XMP ", b"<x/>"),
        ]);
        let meta = webp_meta(&file, MetaPolicy::KeepAll);
        for n in 0..file.len() {
            let cut = &file[..n];
            let _ = webp_chunks(cut);
            let _ = webp_is_lossless(cut);
            let _ = webp_orientation(cut);
            let _ = webp_meta(cut, MetaPolicy::KeepAll);
            let _ = webp_assemble(cut, &meta, 192, 128);
        }
        // Deterministic pseudo-random garbage as well as truncations.
        let mut state = 0x12345678u32;
        let mut next = || {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 8) as u8
        };
        for _ in 0..200 {
            let len = usize::from(next()) % 40;
            let junk: Vec<u8> = (0..len).map(|_| next()).collect();
            let _ = webp_chunks(&junk);
            let _ = webp_is_lossless(&junk);
            let _ = webp_orientation(&junk);
            let _ = webp_meta(&junk, MetaPolicy::StripKeepIcc);
            let _ = webp_assemble(&junk, &meta, 0, 0);
        }
    }
}

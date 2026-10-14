//! End-to-end behavior of `compress` on the committed fixtures.

use squish_core::{Effort, Error, Format, MetaPolicy, Options, Quality, compress, detect, score};

const FIXTURES: &[&str] = &[
    "photo-rgb.png",
    "photo-gray.png",
    "photo-icc.png",
    "logo-alpha.png",
    "palette.png",
    "photo.jpg",
    "photo-meta.jpg",
    "photo-gray.jpg",
    "photo.webp",
    "photo-lossless.webp",
    "logo-alpha.webp",
    "photo-meta.webp",
];

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

struct Decoded {
    width: u32,
    height: u32,
    /// RGBA for PNG, 1 or 3 channels for JPEG.
    pixels: Vec<u8>,
    channels: usize,
}

fn decode(bytes: &[u8]) -> Decoded {
    match detect(bytes).expect("a known format") {
        Format::Png => {
            let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().expect("png header");
            let mut buf = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut buf).expect("png data");
            buf.truncate(info.buffer_size());
            let pixels: Vec<u8> = match info.color_type {
                png::ColorType::Rgba => buf,
                png::ColorType::Rgb => buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
                png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
                png::ColorType::GrayscaleAlpha => buf.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
                png::ColorType::Indexed => unreachable!("expanded"),
            };
            Decoded { width: info.width, height: info.height, pixels, channels: 4 }
        }
        Format::Jpeg => {
            let d = mozjpeg::Decompress::new_mem(bytes).expect("jpeg header");
            let (width, height) = d.size();
            let gray = d.color_space() == mozjpeg::ColorSpace::JCS_GRAYSCALE;
            let mut s = if gray { d.grayscale().unwrap() } else { d.rgb().unwrap() };
            let pixels = s.read_scanlines::<u8>().expect("jpeg data");
            Decoded { width: width as u32, height: height as u32, pixels, channels: if gray { 1 } else { 3 } }
        }
        Format::WebP => {
            let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).expect("webp header");
            let (width, height) = decoder.dimensions();
            let alpha = decoder.has_alpha();
            let mut buf = vec![0; decoder.output_buffer_size().unwrap()];
            decoder.read_image(&mut buf).expect("webp data");
            let pixels = if alpha { buf } else { buf.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
            Decoded { width, height, pixels, channels: 4 }
        }
        other => panic!("fixture in unexpected format {other}"),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn options(quality: Quality, effort: Effort) -> Options {
    Options { quality, effort, ..Options::default() }
}

#[test]
fn fixtures_are_detected_by_content() {
    for name in FIXTURES {
        let expected = if name.ends_with(".png") {
            Format::Png
        } else if name.ends_with(".webp") {
            Format::WebP
        } else {
            Format::Jpeg
        };
        assert_eq!(detect(&fixture(name)), Some(expected), "{name}");
    }
}

#[test]
fn every_setting_keeps_dimensions_and_never_grows_a_file() {
    let settings = [
        options(Quality::Auto, Effort::Default),
        options(Quality::Auto, Effort::Fast),
        options(Quality::Auto, Effort::Max),
        options(Quality::Fixed(60), Effort::Default),
        options(Quality::Lossless, Effort::Default),
    ];
    for name in FIXTURES {
        let input = fixture(name);
        let before = decode(&input);
        for o in &settings {
            let out = compress(&input, o).unwrap_or_else(|e| panic!("{name} with {o:?}: {e}"));
            assert!(out.bytes.len() <= input.len(), "{name} grew with {o:?}");
            assert_eq!(out.original_size, input.len());
            assert_eq!(out.kept_original, out.bytes == input, "{name} kept_original disagrees with the bytes");
            let after = decode(&out.bytes);
            assert_eq!((after.width, after.height), (before.width, before.height), "{name}");
            assert_eq!(after.channels, before.channels, "{name} changed color model");
        }
    }
}

#[test]
fn output_is_deterministic() {
    for name in ["photo.jpg", "photo-rgb.png"] {
        let input = fixture(name);
        let a = compress(&input, &Options::default()).unwrap();
        let b = compress(&input, &Options::default()).unwrap();
        assert_eq!(a.bytes, b.bytes, "{name}");
        assert_eq!(a.report, b.report, "{name}");
    }
}

#[test]
fn a_wasteful_jpeg_shrinks_and_reports_what_it_chose() {
    let input = fixture("photo.jpg");
    let out = compress(&input, &Options::default()).unwrap();
    assert!(!out.kept_original);
    assert!(out.bytes.len() * 100 < input.len() * 95, "{} -> {}, {:?}", input.len(), out.bytes.len(), out.report);
    let r = &out.report;
    assert!((30..=95).contains(&r.setting.unwrap()), "{r:?}");
    assert!(r.score.unwrap() >= r.target.unwrap(), "{r:?}");
    assert!((2..=8).contains(&r.probes), "{r:?}");
}

#[test]
fn every_reported_score_is_the_score_of_the_output() {
    // `--effort max` re-encodes after scoring, and a JPEG search can ship a probe other
    // than the winner. The report has to describe the bytes that ship either way.
    for name in FIXTURES {
        let input = fixture(name);
        for effort in [Effort::Default, Effort::Max] {
            let out = compress(&input, &options(Quality::Auto, effort)).unwrap();
            let Some(reported) = out.report.score else { continue };
            if out.kept_original {
                continue;
            }
            let measured = score(&input, &out.bytes).unwrap();
            assert!((measured - reported).abs() < 1e-6, "{name} {effort:?}: reported {reported}, measured {measured}");
        }
    }
}

/// The pixels of `photo.jpg` encoded by mozjpeg at quality `q`, already trellis-optimized.
fn mozjpeg_at(q: f32) -> Vec<u8> {
    let photo = decode(&fixture("photo.jpg"));
    let mut c = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    c.set_size(photo.width as usize, photo.height as usize);
    c.set_quality(q);
    let mut started = c.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&photo.pixels).unwrap();
    started.finish().unwrap()
}

#[test]
fn a_jpeg_whose_winner_is_too_big_ships_the_best_probe_that_fits() {
    // At q95 the trellis winner comes out bigger than this already-optimized input, and
    // only a probe below the target fits. That probe ships instead of the input.
    let input = mozjpeg_at(95.0);
    let out = compress(&input, &Options::default()).unwrap();
    let r = &out.report;
    assert!(!out.kept_original && out.bytes.len() < input.len(), "{} -> {}, {r:?}", input.len(), out.bytes.len());
    assert!(r.score.unwrap() < r.target.unwrap(), "expected the fallback probe, got {r:?}");
    assert!(r.setting.unwrap() < 95, "{r:?}");
}

#[test]
fn a_jpeg_that_nothing_beats_is_kept() {
    let input = mozjpeg_at(60.0);
    let out = compress(&input, &Options::default()).unwrap();
    assert!(out.kept_original, "{:?}", out.report);
    assert_eq!(out.bytes, input);
}

#[test]
fn effort_max_is_not_bigger_than_default() {
    let input = fixture("photo.jpg");
    let default = compress(&input, &options(Quality::Auto, Effort::Default)).unwrap();
    let max = compress(&input, &options(Quality::Auto, Effort::Max)).unwrap();
    assert!(max.bytes.len() <= default.bytes.len());
}

#[test]
fn effort_fast_skips_the_search() {
    let out = compress(&fixture("photo.jpg"), &options(Quality::Auto, Effort::Fast)).unwrap();
    assert_eq!(out.report.setting, Some(66));
    assert_eq!(out.report.probes, 0);
    assert_eq!(out.report.score, None);
}

#[test]
fn lossless_jpeg_changes_no_pixels() {
    let input = fixture("photo-meta.jpg");
    let out = compress(&input, &options(Quality::Lossless, Effort::Default)).unwrap();
    assert!(out.bytes.len() < input.len(), "metadata should have been removed");
    assert_eq!(decode(&out.bytes).pixels, decode(&input).pixels);
}

#[test]
fn jpeg_default_policy_keeps_icc_and_orientation_and_drops_the_rest() {
    let out = compress(&fixture("photo-meta.jpg"), &Options::default()).unwrap();
    let b = &out.bytes;
    assert!(contains(b, b"ICC_PROFILE\0"), "ICC profile lost");
    // Orientation 6 in a minimal big-endian Exif block.
    assert!(contains(b, &[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1, 0, 6]), "orientation lost");
    assert!(!contains(b, b"GPSLatitude"), "GPS text survived");
    assert!(!contains(b, b"fixture generator"), "comment survived");
}

#[test]
fn jpeg_strip_all_drops_icc_and_orientation() {
    let o = Options { metadata: MetaPolicy::StripAll, ..Options::default() };
    let b = compress(&fixture("photo-meta.jpg"), &o).unwrap().bytes;
    assert!(!contains(&b, b"ICC_PROFILE\0") && !contains(&b, b"Exif\0\0"));
}

#[test]
fn jpeg_keep_all_keeps_exif_and_comment() {
    let o = Options { metadata: MetaPolicy::KeepAll, ..Options::default() };
    let b = compress(&fixture("photo-meta.jpg"), &o).unwrap().bytes;
    assert!(contains(&b, b"GPSLatitude") && contains(&b, b"fixture generator") && contains(&b, b"ICC_PROFILE\0"));
}

#[test]
fn webp_metadata_follows_the_policy() {
    let input = fixture("photo-meta.webp");
    let keep = compress(&input, &Options::default()).unwrap();
    assert!(!keep.kept_original, "q95 input must shrink at q71: {:?}", keep.report);
    let b = &keep.bytes;
    assert!(contains(b, b"ICCP"), "default policy must keep the profile");
    // Orientation 6 in a minimal EXIF block holding bare TIFF data.
    assert!(contains(b, &[0x01, 0x12, 0x00, 0x03, 0, 0, 0, 1, 0, 6]), "orientation lost");
    assert!(!contains(b, b"xmpmeta"), "XMP survived");
    assert!(!contains(b, b"ZZZZ"), "unknown chunk survived");

    let o = Options { metadata: MetaPolicy::StripAll, ..Options::default() };
    let b = compress(&input, &o).unwrap().bytes;
    assert!(!contains(&b, b"ICCP") && !contains(&b, b"EXIF"));

    let o = Options { metadata: MetaPolicy::KeepAll, ..Options::default() };
    let b = compress(&input, &o).unwrap().bytes;
    assert!(contains(&b, b"ICCP") && contains(&b, b"ZZZZ") && contains(&b, b"xmpmeta"), "keep-all lost a chunk");
    assert!(contains(&b, b"GPSLatitude"), "keep-all dropped the EXIF body");
}

#[test]
fn webp_lossless_stays_lossless_at_every_setting() {
    let input = fixture("photo-lossless.webp");
    let before = decode(&input);
    for quality in [Quality::Auto, Quality::Fixed(50), Quality::Lossless] {
        for effort in [Effort::Fast, Effort::Default, Effort::Max] {
            let out = compress(&input, &options(quality, effort)).unwrap();
            let label = format!("{quality:?} {effort:?}");
            assert!(contains(&out.bytes, b"VP8L"), "{label} turned lossless into lossy");
            assert_eq!(decode(&out.bytes).pixels, before.pixels, "{label} changed pixels");
        }
    }
}

#[test]
fn grayscale_jpeg_stays_grayscale() {
    let out = compress(&fixture("photo-gray.jpg"), &Options::default()).unwrap();
    assert_eq!(decode(&out.bytes).channels, 1);
}

#[test]
fn png_photo_gets_much_smaller_and_reports_its_palette() {
    let input = fixture("photo-rgb.png");
    let out = compress(&input, &Options::default()).unwrap();
    assert!(!out.kept_original);
    assert!(out.bytes.len() * 10 < input.len() * 8, "{} -> {}", input.len(), out.bytes.len());
    let colors = out.report.setting.unwrap();
    assert!((2..=256).contains(&colors), "{:?}", out.report);
    // The output really is a palette image: color type 3 in IHDR.
    assert_eq!(out.bytes[25], 3);
}

#[test]
fn png_search_meets_the_target_below_256_colors() {
    for name in ["photo-gray.png", "logo-alpha.png", "palette.png"] {
        let r = compress(&fixture(name), &Options::default()).unwrap().report;
        assert!(r.score.unwrap() >= r.target.unwrap(), "{name}: {r:?}");
        assert!(r.setting.unwrap() < 256, "{name}: {r:?}");
    }
    // Noisy color photos miss even at 256 colors (ADR-0022). The search must stop at the top.
    let r = compress(&fixture("photo-rgb.png"), &Options::default()).unwrap().report;
    assert_eq!((r.setting, r.probes), (Some(256), 1), "{r:?}");
}

#[test]
fn png_transparency_survives() {
    let input = fixture("logo-alpha.png");
    let out = compress(&input, &Options::default()).unwrap();
    let (before, after) = (decode(&input), decode(&out.bytes));
    let alpha = |d: &Decoded, x: usize, y: usize| d.pixels[(y * d.width as usize + x) * 4 + 3];
    assert_eq!(alpha(&after, 0, 0), 0, "corner must stay transparent");
    assert_eq!(alpha(&after, 48, 48), 255, "center must stay opaque");
    // Soft edge pixels keep roughly the alpha they had.
    let worst = (0..before.pixels.len() / 4)
        .map(|i| (i32::from(before.pixels[i * 4 + 3]) - i32::from(after.pixels[i * 4 + 3])).abs())
        .max()
        .unwrap();
    assert!(worst < 80, "alpha moved by {worst}");
}

#[test]
fn png_lossless_keeps_every_pixel() {
    for name in ["photo-rgb.png", "logo-alpha.png", "photo-gray.png"] {
        let input = fixture(name);
        let out = compress(&input, &options(Quality::Lossless, Effort::Default)).unwrap();
        assert_eq!(decode(&out.bytes).pixels, decode(&input).pixels, "{name}");
        assert_eq!(out.report.score, None);
    }
}

#[test]
fn png_icc_follows_the_policy() {
    let input = fixture("photo-icc.png");
    assert!(contains(&input, b"iCCP"));
    let keep = compress(&input, &Options::default()).unwrap().bytes;
    assert!(contains(&keep, b"iCCP"), "default policy must keep the profile");
    let strip = compress(&input, &Options { metadata: MetaPolicy::StripAll, ..Options::default() }).unwrap().bytes;
    assert!(!contains(&strip, b"iCCP"));
    for q in [Quality::Auto, Quality::Lossless] {
        let o = Options { quality: q, ..Options::default() };
        assert!(contains(&compress(&input, &o).unwrap().bytes, b"iCCP"), "{q:?}");
    }
}

/// An RGB PNG of `pixels`, built with the `png` crate.
fn rgb_png(width: u32, height: u32, pixels: &[u8], setup: impl FnOnce(&mut png::Encoder<&mut Vec<u8>>)) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        setup(&mut encoder);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }
    out
}

/// The first `kind` chunk of `png`, with its length and CRC.
fn raw_chunk<'a>(png: &'a [u8], kind: &[u8; 4]) -> &'a [u8] {
    let at = png.windows(4).position(|w| w == kind).expect("chunk present") - 4;
    let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
    &png[at..at + 12 + len]
}

/// IHDR color type of a PNG.
fn color_type(png: &[u8]) -> u8 {
    png[25]
}

#[test]
fn png_color_chunks_survive_the_default_policy() {
    // gAMA 1/1.8, the color space of a PNG without an ICC profile.
    let photo = decode(&fixture("photo-rgb.png"));
    let rgb: Vec<u8> = photo.pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let input = rgb_png(photo.width, photo.height, &rgb, |e| e.set_source_gamma(png::ScaledFloat::new(1.0 / 1.8)));
    for q in [Quality::Auto, Quality::Lossless] {
        let o = Options { quality: q, ..Options::default() };
        assert!(contains(&compress(&input, &o).unwrap().bytes, b"gAMA"), "{q:?}");
    }
    let strip = Options { metadata: MetaPolicy::StripAll, ..Options::default() };
    assert!(!contains(&compress(&input, &strip).unwrap().bytes, b"gAMA"));
}

#[test]
fn png_icc_profile_stays_on_a_color_image() {
    // Gray content stored as RGB with an RGB profile: reducing the output to grayscale
    // would leave a profile that doesn't fit its color type.
    let gray = decode(&fixture("photo-gray.png"));
    let rgb: Vec<u8> = gray.pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let plain = rgb_png(gray.width, gray.height, &rgb, |_| {});
    let with_icc = fixture("photo-icc.png");
    let input = [&plain[..33], raw_chunk(&with_icc, b"iCCP"), &plain[33..]].concat();
    for q in [Quality::Auto, Quality::Lossless] {
        let out = compress(&input, &options(q, Effort::Default)).unwrap();
        assert!(contains(&out.bytes, b"iCCP"), "{q:?}");
        assert!(matches!(color_type(&out.bytes), 2 | 3 | 6), "{q:?} gave color type {}", color_type(&out.bytes));
    }
}

/// A smooth pattern with thousands of colors: dithering it to 256 scores badly and
/// compresses worse than the exact pixels.
fn gradient(setup: impl FnOnce(&mut png::Encoder<&mut Vec<u8>>)) -> Vec<u8> {
    let (w, h) = (300u32, 200u32);
    let rgb: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [x as u8, (y * 2) as u8, (x * y) as u8])).collect();
    rgb_png(w, h, &rgb, setup)
}

#[test]
fn png_that_misses_at_256_colors_takes_lossless_when_smaller() {
    let well_compressed = gradient(|e| {
        e.set_compression(png::Compression::High);
        e.set_filter(png::Filter::Adaptive);
    });
    let poorly_compressed = gradient(|e| {
        e.set_compression(png::Compression::NoCompression);
        e.set_filter(png::Filter::NoFilter);
    });
    let settings = [
        options(Quality::Auto, Effort::Default),
        options(Quality::Auto, Effort::Fast),
        options(Quality::Fixed(100), Effort::Default),
    ];
    for input in [&well_compressed, &poorly_compressed] {
        for o in &settings {
            let out = compress(input, o).unwrap();
            let label = format!("{} bytes in, {:?} {:?}", input.len(), o.quality, o.effort);
            assert!(!out.kept_original, "{label}");
            assert_eq!(
                decode(&out.bytes).pixels,
                decode(input).pixels,
                "{label}: lossless output must keep every pixel"
            );
            assert_eq!((out.report.setting, out.report.score), (None, Some(100.0)), "{label}: {:?}", out.report);
        }
    }
}

/// CRC-32 as PNG chunks use it.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let body = [&kind[..], data].concat();
    [&(data.len() as u32).to_be_bytes()[..], &body, &crc32(&body).to_be_bytes()].concat()
}

#[test]
fn hdr_png_only_gets_the_lossless_steps() {
    // cICP with BT.2020 primaries and the PQ transfer function.
    let photo = decode(&fixture("photo-rgb.png"));
    let rgb: Vec<u8> = photo.pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let plain = rgb_png(photo.width, photo.height, &rgb, |_| {});
    let input = [&plain[..33], &png_chunk(b"cICP", &[9, 16, 0, 1]), &plain[33..]].concat();
    for q in [Quality::Auto, Quality::Fixed(60)] {
        let out = compress(&input, &options(q, Effort::Default)).unwrap();
        assert_eq!(decode(&out.bytes).pixels, photo.pixels, "{q:?} changed pixels");
        assert_eq!(out.report, squish_core::Report::default(), "{q:?}");
        assert!(contains(&out.bytes, b"cICP"), "{q:?} dropped the HDR tag");
    }
}

/// `photo.jpg`'s pixels as a CMYK JPEG, with the Adobe marker libjpeg writes for CMYK.
fn cmyk_jpeg() -> Vec<u8> {
    let photo = decode(&fixture("photo.jpg"));
    let cmyk: Vec<u8> = photo
        .pixels
        .chunks(3)
        .flat_map(|p| {
            let k = 255 - p[0].max(p[1]).max(p[2]);
            [255 - p[0] - k, 255 - p[1] - k, 255 - p[2] - k, k]
        })
        .collect();
    let mut c = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_CMYK);
    c.set_size(photo.width as usize, photo.height as usize);
    c.set_quality(95.0);
    let mut started = c.start_compress(Vec::new()).unwrap();
    started.write_marker(mozjpeg::Marker::COM, b"cmyk fixture");
    started.write_scanlines(&cmyk).unwrap();
    started.finish().unwrap()
}

fn cmyk_pixels(bytes: &[u8]) -> Vec<u8> {
    let d = mozjpeg::Decompress::new_mem(bytes).unwrap();
    assert_eq!(d.color_space(), mozjpeg::ColorSpace::JCS_CMYK);
    d.to_colorspace(mozjpeg::ColorSpace::JCS_CMYK).unwrap().read_scanlines::<u8>().unwrap()
}

#[test]
fn cmyk_jpeg_only_gets_the_lossless_steps() {
    let input = cmyk_jpeg();
    for o in [Options::default(), options(Quality::Fixed(60), Effort::Fast)] {
        let out = compress(&input, &o).unwrap();
        assert!(out.bytes.len() < input.len(), "{o:?}: the comment should have been stripped");
        assert_eq!(cmyk_pixels(&out.bytes), cmyk_pixels(&input), "{o:?} changed pixels");
        assert!(contains(&out.bytes, b"Adobe"), "{o:?} dropped the Adobe marker");
        assert_eq!(out.report, squish_core::Report::default());
    }
}

/// Layouts no fixture covers, built here so they stay tiny.
fn unusual_inputs() -> Vec<(&'static str, Vec<u8>)> {
    let (w, h) = (64u32, 48u32);
    let png_of = |color, depth, pixels: Vec<u8>| {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(color);
            e.set_depth(depth);
            let mut writer = e.write_header().unwrap();
            writer.write_image_data(&pixels).unwrap();
        }
        out
    };
    let ramp = |x: u32, y: u32| ((x * 4 + y * 3) % 256) as u8;
    let rgb16: Vec<u8> =
        (0..h).flat_map(|y| (0..w).flat_map(move |x| [ramp(x, y), x as u8, y as u8, 7, ramp(y, x), 0])).collect();
    let gray16: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [ramp(x, y), (x * y) as u8])).collect();
    let gray_alpha: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| [ramp(x, y), (x * 4) as u8])).collect();
    let one_bit: Vec<u8> =
        (0..h).flat_map(|y| (0..w / 8).map(move |x| if (x + y) % 2 == 0 { 0xAA } else { 0x55 })).collect();

    let photo = decode(&fixture("photo.jpg"));
    let mut c = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    c.set_size(photo.width as usize, photo.height as usize);
    c.set_quality(92.0);
    c.set_progressive_mode();
    let mut started = c.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&photo.pixels).unwrap();
    let progressive = started.finish().unwrap();

    let mut c = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    c.set_size(1, 1);
    let mut started = c.start_compress(Vec::new()).unwrap();
    started.write_scanlines(&[200, 100, 50]).unwrap();
    let tiny_jpeg = started.finish().unwrap();

    vec![
        ("16-bit rgb", png_of(png::ColorType::Rgb, png::BitDepth::Sixteen, rgb16)),
        ("16-bit gray", png_of(png::ColorType::Grayscale, png::BitDepth::Sixteen, gray16)),
        ("gray+alpha", png_of(png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, gray_alpha)),
        ("1-bit gray", png_of(png::ColorType::Grayscale, png::BitDepth::One, one_bit)),
        ("1x1 png", rgb_png(1, 1, &[200, 100, 50], |_| {})),
        ("progressive jpeg", progressive),
        ("1x1 jpeg", tiny_jpeg),
    ]
}

#[test]
fn unusual_layouts_keep_dimensions_and_never_grow() {
    let settings = [
        options(Quality::Auto, Effort::Default),
        options(Quality::Auto, Effort::Fast),
        options(Quality::Fixed(60), Effort::Default),
        options(Quality::Lossless, Effort::Default),
    ];
    for (name, input) in unusual_inputs() {
        let before = decode(&input);
        for o in &settings {
            let out = compress(&input, o).unwrap_or_else(|e| panic!("{name} with {o:?}: {e}"));
            assert!(out.bytes.len() <= input.len(), "{name} grew with {o:?}");
            let after = decode(&out.bytes);
            assert_eq!((after.width, after.height), (before.width, before.height), "{name} with {o:?}");
            if o.quality == Quality::Lossless {
                assert_eq!(after.pixels, before.pixels, "{name}: lossless changed pixels");
            }
        }
    }
}

#[test]
fn animated_png_is_reported_not_touched() {
    let err = compress(&fixture("animated.png"), &Options::default()).unwrap_err();
    assert_eq!(err, Error::Animated(Format::Png));
    assert_eq!(err.code(), "animated_image");
}

#[test]
fn animated_webp_is_reported_not_touched() {
    let err = compress(&fixture("animated.webp"), &Options::default()).unwrap_err();
    assert_eq!(err, Error::Animated(Format::WebP));
    assert_eq!(err.code(), "animated_image");
}

#[test]
fn unknown_and_unbuilt_formats_are_unsupported() {
    let gif = compress(b"GIF89a\x01\x00\x01\x00\x00\x00\x00;", &Options::default()).unwrap_err();
    assert_eq!(gif, Error::Unsupported(None));
    let mut avif = 24u32.to_be_bytes().to_vec();
    avif.extend_from_slice(b"ftypavif\0\0\0\0avifmif1");
    avif.extend_from_slice(&[0; 8]);
    assert_eq!(compress(&avif, &Options::default()).unwrap_err(), Error::Unsupported(Some(Format::Avif)));
    assert_eq!(compress(b"", &Options::default()).unwrap_err(), Error::Unsupported(None));
}

#[test]
fn pixel_limit_is_checked_from_the_header() {
    let o = Options { max_pixels: 1000, ..Options::default() };
    for name in ["photo.jpg", "photo-rgb.png"] {
        match compress(&fixture(name), &o) {
            Err(Error::TooManyPixels { width: 192, height: 128, limit: 1000 }) => {}
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn damaged_files_fail_cleanly() {
    let jpeg = fixture("photo.jpg");
    let png = fixture("photo-rgb.png");
    let garbage_jpeg = [&[0xFF, 0xD8, 0xFF, 0xE0][..], &[0x55; 64]].concat();
    let garbage_png = [&png[..33], &[0x55; 64]].concat();
    for (name, bytes) in [
        ("garbage jpeg", garbage_jpeg),
        ("garbage png", garbage_png),
        ("png cut in half", png[..png.len() / 2].to_vec()),
        ("png header only", png[..33].to_vec()),
        ("jpeg header only", jpeg[..200].to_vec()),
    ] {
        match compress(&bytes, &Options::default()) {
            Err(Error::Decode(_)) | Err(Error::Encode(_)) => {}
            Ok(out) => assert!(out.bytes.len() <= bytes.len(), "{name}"),
            Err(other) => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn every_prefix_of_a_jpeg_is_survivable() {
    let jpeg = fixture("photo-gray.jpg");
    for n in (0..jpeg.len()).step_by(97) {
        let _ = compress(&jpeg[..n], &options(Quality::Auto, Effort::Fast));
    }
}

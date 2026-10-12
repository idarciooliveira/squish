//! Phase 0 quality-matched sweep.
//!
//! For each file, find the cheapest setting whose dssim is at or below TinyPNG's
//! (with a floor, so differences nobody can see don't count), then report bytes.
//! PNG: smallest max_colors for quantizr, and for imagequant as a reference.
//! JPEG: lowest quality for mozjpeg and for jpegli.
//! Results go to a CSV: file,format,variant,setting,out_bytes,dssim,target,tinypng_bytes,met,encode_ms.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use dssim::Dssim;
use image::GenericImageView;
use rgb::{FromSlice, RGBA8};

type Res<T> = Result<T, Box<dyn Error>>;

/// dssim below this counts as equal to TinyPNG, whatever TinyPNG scored.
const FLOOR: f64 = 0.0002;

struct Found {
    variant: &'static str,
    setting: String,
    bytes: Vec<u8>,
    dssim: f64,
    met: bool,
    ms: u128,
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "../images".into()));
    let tiny_dir = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "../tinypng".into()));
    let out_csv = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| "../results/sweep.csv".into()));

    let mut files: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.file_name().unwrap_or_default() != ".gitkeep")
        .collect();
    files.sort();

    let ds = Dssim::new();
    let mut rows = vec!["file,format,variant,setting,out_bytes,dssim,target,tinypng_bytes,met,encode_ms".to_string()];

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let orig = fs::read(path)?;
        let Ok(tiny) = fs::read(tiny_dir.join(&name)) else {
            println!("--- {name}: no TinyPNG output, skipped");
            continue;
        };
        let fmt = if orig.starts_with(b"\x89PNG") { "png" } else if orig.starts_with(&[0xFF, 0xD8]) { "jpeg" } else { continue };
        let img = image::load_from_memory(&orig)?;
        let (w, h) = img.dimensions();
        let rgba = img.to_rgba8();
        let reference = ds
            .create_image_rgba(rgba.as_raw().as_rgba(), w as usize, h as usize)
            .ok_or("dssim rejected original")?;
        let score_rgba = |px: &[RGBA8]| -> f64 {
            let i = ds.create_image_rgba(px, w as usize, h as usize).expect("dssim image");
            f64::from(ds.compare(&reference, i).0)
        };
        let score_bytes = |b: &[u8]| -> f64 {
            let d = image::load_from_memory(b).expect("decode candidate").to_rgba8();
            score_rgba(d.as_raw().as_rgba())
        };
        let tiny_dssim = score_bytes(&tiny);
        let target = tiny_dssim.max(FLOOR);
        println!("--- {name} ({fmt}) tinypng {} bytes dssim {tiny_dssim:.5} target {target:.5}", tiny.len());

        // SWEEP_ONLY=quantizr limits the run to quantizr on PNGs, SWEEP_ONLY=jpeg to JPEGs.
        let only = std::env::var("SWEEP_ONLY").unwrap_or_default();
        let only_quantizr = only == "quantizr";
        let found: Vec<Found> = if fmt == "png" && only == "jpeg" {
            vec![]
        } else if fmt == "png" {
            let mut f = vec![png_search("quantizr+oxipng", &rgba, w, h, target, &score_rgba, quantizr_quantize)];
            if !only_quantizr {
                f.push(png_search("imagequant+oxipng", &rgba, w, h, target, &score_rgba, imagequant_quantize));
            }
            f
        } else if only_quantizr {
            vec![]
        } else {
            vec![
                jpeg_search("mozjpeg", mozjpeg_encode, &rgba, w, h, target, &score_bytes),
                jpeg_search("jpegli", jpegli_encode, &rgba, w, h, target, &score_bytes),
            ]
        };

        for f in found {
            println!(
                "  {:18} {:>8} -> {:>9} bytes dssim {:.5} {}",
                f.variant, f.setting, f.bytes.len(), f.dssim, if f.met { "met" } else { "MISSED" }
            );
            rows.push(
                [
                    csv_field(&name),
                    fmt.into(),
                    f.variant.into(),
                    f.setting,
                    f.bytes.len().to_string(),
                    format!("{:.5}", f.dssim),
                    format!("{target:.5}"),
                    tiny.len().to_string(),
                    f.met.to_string(),
                    f.ms.to_string(),
                ]
                .join(","),
            );
        }
    }

    fs::write(&out_csv, rows.join("\n") + "\n")?;
    println!("wrote {}", out_csv.display());
    Ok(())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Quantizes to at most `colors` colors. Returns an RGBA palette and one index per pixel.
type Quantize = fn(&image::RgbaImage, u32, u32, u32) -> Option<(Vec<RGBA8>, Vec<u8>)>;

fn quantizr_quantize(rgba: &image::RgbaImage, w: u32, h: u32, colors: u32) -> Option<(Vec<RGBA8>, Vec<u8>)> {
    let img = quantizr::Image::new(rgba.as_raw(), w as usize, h as usize).ok()?;
    let mut opts = quantizr::Options::default();
    opts.set_max_colors(colors as i32).ok()?;
    let mut res = quantizr::QuantizeResult::quantize(&img, &opts);
    res.set_dithering_level(quantizr_dither()).ok()?;
    let mut idx = vec![0u8; (w * h) as usize];
    res.remap_image(&img, &mut idx).ok()?;
    let p = res.get_palette();
    let pal = p.entries[..p.count as usize].iter().map(|c| RGBA8::new(c.r, c.g, c.b, c.a)).collect();
    Some((pal, idx))
}

/// Dithering level for quantizr, from `QUANTIZR_DITHER` (default 1.0).
fn quantizr_dither() -> f32 {
    std::env::var("QUANTIZR_DITHER").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn imagequant_quantize(rgba: &image::RgbaImage, w: u32, h: u32, colors: u32) -> Option<(Vec<RGBA8>, Vec<u8>)> {
    let px: Vec<imagequant::RGBA> = rgba.pixels().map(|p| imagequant::RGBA::new(p[0], p[1], p[2], p[3])).collect();
    let mut liq = imagequant::new();
    liq.set_speed(4).ok()?;
    liq.set_quality(0, 100).ok()?;
    liq.set_max_colors(colors).ok()?;
    let mut img = liq.new_image(&px[..], w as usize, h as usize, 0.0).ok()?;
    let mut res = liq.quantize(&mut img).ok()?;
    res.set_dithering_level(1.0).ok()?;
    let (pal, idx) = res.remapped(&mut img).ok()?;
    Some((pal.iter().map(|c| RGBA8::new(c.r, c.g, c.b, c.a)).collect(), idx))
}

/// Binary search for the smallest palette that meets `target`. Falls back to 256 colors.
fn png_search(
    variant: &'static str,
    rgba: &image::RgbaImage,
    w: u32,
    h: u32,
    target: f64,
    score: &dyn Fn(&[RGBA8]) -> f64,
    quantize: Quantize,
) -> Found {
    let t = Instant::now();
    let try_colors = |n: u32| -> Option<(Vec<RGBA8>, Vec<u8>, f64)> {
        let (pal, idx) = quantize(rgba, w, h, n)?;
        let px: Vec<RGBA8> = idx.iter().map(|&i| pal[i as usize]).collect();
        let d = score(&px);
        Some((pal, idx, d))
    };
    let (mut lo, mut hi) = (2u32, 256u32);
    let mut best = try_colors(hi);
    let met = best.as_ref().is_some_and(|b| b.2 <= target);
    if met {
        while lo < hi {
            let mid = (lo + hi) / 2;
            match try_colors(mid) {
                Some(c) if c.2 <= target => {
                    hi = mid;
                    best = Some(c);
                }
                _ => lo = mid + 1,
            }
        }
    }
    let Some((pal, idx, d)) = best else {
        return Found { variant, setting: "failed".into(), bytes: vec![], dssim: f64::NAN, met: false, ms: t.elapsed().as_millis() };
    };
    let bytes = encode_indexed_oxipng(w, h, &pal, &idx).unwrap_or_default();
    Found { variant, setting: format!("{}c", pal.len()), bytes, dssim: d, met, ms: t.elapsed().as_millis() }
}

fn encode_indexed_oxipng(w: u32, h: u32, pal: &[RGBA8], idx: &[u8]) -> Res<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(pal.iter().flat_map(|c| [c.r, c.g, c.b]).collect::<Vec<u8>>());
        if pal.iter().any(|c| c.a != 255) {
            enc.set_trns(pal.iter().map(|c| c.a).collect::<Vec<u8>>());
        }
        enc.write_header()?.write_image_data(idx)?;
    }
    Ok(oxipng::optimize_from_memory(&buf, &oxipng::Options::from_preset(2))?)
}

fn mozjpeg_encode(rgb: &image::RgbImage, w: u32, h: u32, q: u8) -> Option<Vec<u8>> {
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    comp.set_size(w as usize, h as usize);
    comp.set_quality(q as f32);
    comp.set_progressive_mode();
    let mut comp = comp.start_compress(Vec::new()).ok()?;
    comp.write_scanlines(&rgb.as_raw()[..]).ok()?;
    comp.finish().ok()
}

fn jpegli_encode(rgb: &image::RgbImage, w: u32, h: u32, q: u8) -> Option<Vec<u8>> {
    let mut comp = jpegli::Compress::new(jpegli::ColorSpace::JCS_RGB);
    comp.set_size(w as usize, h as usize);
    comp.set_quality(q as f32);
    comp.set_progressive_mode();
    comp.set_optimize_coding(true);
    // JPEGLI_420=1 forces 4:2:0 chroma, like mozjpeg's default.
    if std::env::var("JPEGLI_420").is_ok_and(|v| v == "1") {
        comp.set_chroma_sampling_pixel_sizes((2, 2), (2, 2));
    }
    let mut comp = comp.start_compress(Vec::new()).ok()?;
    comp.write_scanlines(&rgb.as_raw()[..]).ok()?;
    comp.finish().ok()
}

type JpegEncode = fn(&image::RgbImage, u32, u32, u8) -> Option<Vec<u8>>;

/// Binary search for the lowest quality that meets `target`. Falls back to q95.
fn jpeg_search(
    variant: &'static str,
    encode: JpegEncode,
    rgba: &image::RgbaImage,
    w: u32,
    h: u32,
    target: f64,
    score: &dyn Fn(&[u8]) -> f64,
) -> Found {
    let t = Instant::now();
    let rgb = image::DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
    let try_q = |q: u8| encode(&rgb, w, h, q).map(|b| { let d = score(&b); (q, b, d) });
    let (mut lo, mut hi) = (20u8, 95u8);
    let mut best = try_q(hi);
    let met = best.as_ref().is_some_and(|b| b.2 <= target);
    if met {
        while lo < hi {
            let mid = (lo + hi) / 2;
            match try_q(mid) {
                Some(c) if c.2 <= target => {
                    hi = mid;
                    best = Some(c);
                }
                _ => lo = mid + 1,
            }
        }
    }
    match best {
        Some((q, bytes, d)) => Found { variant, setting: format!("q{q}"), bytes, dssim: d, met, ms: t.elapsed().as_millis() },
        None => Found { variant, setting: "failed".into(), bytes: vec![], dssim: f64::NAN, met: false, ms: t.elapsed().as_millis() },
    }
}

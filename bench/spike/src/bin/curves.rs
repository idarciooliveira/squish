//! Phase 1 calibration curves.
//!
//! For each file with a TinyPNG output, encodes a fixed grid of settings and scores
//! every result with dssim (the NFR-1 reference) and with each permissive candidate
//! metric, at several scales, timing every step. TinyPNG's own output is scored too,
//! as setting `tinypng`. The grid is what bench/scripts/analyse-curves.py works from:
//! fixed-target tests, metric calibration and search-cost estimates.
//!
//! JPEG grid: mozjpeg quality 30 to 95 in steps of 3, progressive, 4:2:0.
//! PNG grid: quantizr palette sizes 2 to 256, dithering 1.0, then oxipng preset 2.
//!
//! Metrics: dssim (full scale, RGBA), fast-ssim2 (`fs2`), butteraugli max-norm (`ba`)
//! and 3-norm (`ba3`) at scales `1`, `2` (half) and `1mp` (box-downscaled to about
//! one megapixel), image-compare SSIM and hybrid at `1mp`, and rust-av ssimulacra2
//! (`s2rs`) once per file for speed. Images with alpha are composited over white and
//! black for the RGB metrics, and the worse of the two scores is kept. Images above
//! 30 MP are scored at `1mp` only, to fit in memory.
//!
//! Output is long-format CSV: file,format,setting,bytes,metric,scale,value,wall_ms,cpu_ms.
//! cpu_ms is process CPU time, so it counts every thread a metric spreads across.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use dssim::Dssim;
use fast_ssim2::{PixelDescriptor, PixelSlice, Ssimulacra2Reference};
use image::{GenericImageView, RgbImage, RgbaImage};
use rgb::{FromSlice, RGBA8};

type Res<T> = Result<T, Box<dyn Error>>;

const JPEG_GRID: std::ops::RangeInclusive<u8> = 30..=95;
const PNG_GRID: &[u32] = &[2, 4, 8, 12, 16, 24, 32, 48, 64, 96, 128, 160, 192, 224, 256];

fn cpu_ms() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let t = |v: libc::timeval| v.tv_sec as f64 * 1000.0 + v.tv_usec as f64 / 1000.0;
    t(u.ru_utime) + t(u.ru_stime)
}

/// Runs `f`, returning its result with wall and CPU milliseconds.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64, f64) {
    let (w, c) = (Instant::now(), cpu_ms());
    let r = f();
    (r, w.elapsed().as_secs_f64() * 1000.0, cpu_ms() - c)
}

struct Out {
    rows: Vec<String>,
}

impl Out {
    #[allow(clippy::too_many_arguments)]
    fn row(&mut self, file: &str, fmt: &str, setting: &str, bytes: usize, metric: &str, scale: &str, value: Option<f64>, wall: f64, cpu: f64) {
        let v = value.map(|v| format!("{v:.6}")).unwrap_or_default();
        self.rows.push(format!("{},{fmt},{setting},{bytes},{metric},{scale},{v},{wall:.1},{cpu:.1}", csv_field(file)));
    }
}

/// RGB views of one image: one if opaque, composited over white and black if not.
fn views(rgba: &RgbaImage, has_alpha: bool) -> Vec<RgbImage> {
    let bgs: &[u8] = if has_alpha { &[255, 0] } else { &[0] };
    bgs.iter()
        .map(|&bg| {
            let (w, h) = rgba.dimensions();
            let mut out = RgbImage::new(w, h);
            for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
                let a = p[3] as u32;
                for c in 0..3 {
                    o[c] = ((p[c] as u32 * a + bg as u32 * (255 - a) + 127) / 255) as u8;
                }
            }
            out
        })
        .collect()
}

/// Box downscale by an integer factor.
fn downscale(img: &RgbImage, f: u32) -> RgbImage {
    if f <= 1 {
        return img.clone();
    }
    let (w, h) = (img.width() / f, img.height() / f);
    let mut out = RgbImage::new(w.max(8), h.max(8));
    let (w, h) = out.dimensions();
    for y in 0..h {
        for x in 0..w {
            let mut s = [0u32; 3];
            let mut n = 0;
            for dy in 0..f {
                for dx in 0..f {
                    let (sx, sy) = ((x * f + dx).min(img.width() - 1), (y * f + dy).min(img.height() - 1));
                    let p = img.get_pixel(sx, sy);
                    for c in 0..3 {
                        s[c] += p[c] as u32;
                    }
                    n += 1;
                }
            }
            out.put_pixel(x, y, image::Rgb([(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]));
        }
    }
    out
}

fn slice(img: &RgbImage) -> PixelSlice<'_> {
    PixelSlice::new(img.as_raw(), img.width(), img.height(), img.width() as usize * 3, PixelDescriptor::RGB8_SRGB).expect("pixel slice")
}

/// Precomputed references for one scale, one per view.
struct Scale {
    label: &'static str,
    factor: u32,
    fs2: Vec<Ssimulacra2Reference>,
    ba: Vec<butteraugli::ButteraugliReference>,
    refs: Vec<RgbImage>,
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "../images".into()));
    let tiny_dir = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "../tinypng".into()));
    let out_csv = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| "../results/curves.csv".into()));
    // CURVES_ONLY=png or jpeg limits the run to one format.
    let only = std::env::var("CURVES_ONLY").unwrap_or_default();

    let mut files: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.file_name().unwrap_or_default() != ".gitkeep")
        .collect();
    files.sort();

    let ds = Dssim::new();
    let mut out = Out { rows: vec!["file,format,setting,bytes,metric,scale,value,wall_ms,cpu_ms".into()] };

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let orig = fs::read(path)?;
        let Ok(tiny) = fs::read(tiny_dir.join(&name)) else { continue };
        let fmt = if orig.starts_with(b"\x89PNG") { "png" } else if orig.starts_with(&[0xFF, 0xD8]) { "jpeg" } else { continue };
        if !only.is_empty() && only != fmt {
            continue;
        }
        let img = image::load_from_memory(&orig)?;
        let (w, h) = img.dimensions();
        let rgba = img.to_rgba8();
        let has_alpha = fmt == "png" && rgba.pixels().any(|p| p[3] != 255);
        println!("--- {name} ({fmt}, {w}x{h}{})", if has_alpha { ", alpha" } else { "" });
        out.row(&name, fmt, "original", orig.len(), "pixels", "1", Some((w * h) as f64), 0.0, 0.0);

        let (dref, wt, ct) = timed(|| ds.create_image_rgba(rgba.as_raw().as_rgba(), w as usize, h as usize).expect("dssim ref"));
        out.row(&name, fmt, "ref", 0, "dssim", "1", None, wt, ct);

        let base = views(&rgba, has_alpha);
        let f1mp = ((w as f64 * h as f64 / 1.0e6).sqrt().ceil() as u32).max(1);
        // Above 30 MP, full- and half-scale references for every metric don't fit in
        // 16 GB next to dssim's. Only the ~1 MP scale is scored there.
        let big = w as u64 * h as u64 > 30_000_000;
        let mut scales = vec![];
        for (label, factor) in [("1", 1), ("2", 2), ("1mp", f1mp)] {
            if big && label != "1mp" {
                continue;
            }
            let (refs, wt, ct) = timed(|| base.iter().map(|v| downscale(v, factor)).collect::<Vec<_>>());
            out.row(&name, fmt, "ref", 0, "downscale", label, None, wt, ct);
            let (fs2, wt, ct) = timed(|| refs.iter().map(|r| Ssimulacra2Reference::new(&slice(r)).expect("fs2 ref")).collect::<Vec<_>>());
            out.row(&name, fmt, "ref", 0, "fs2", label, None, wt, ct);
            let (ba, wt, ct) = timed(|| {
                refs.iter()
                    .map(|r| butteraugli::ButteraugliReference::new(r.as_raw(), r.width() as usize, r.height() as usize, Default::default()).expect("ba ref"))
                    .collect::<Vec<_>>()
            });
            out.row(&name, fmt, "ref", 0, "ba", label, None, wt, ct);
            scales.push(Scale { label, factor, fs2, ba, refs });
        }

        // Scores one candidate, given as decoded RGBA.
        let mut score = |out: &mut Out, setting: &str, bytes: usize, cand: &RgbaImage, s2rs: bool| {
            let (d, wt, ct) = timed(|| {
                let i = ds.create_image_rgba(cand.as_raw().as_rgba(), w as usize, h as usize).expect("dssim img");
                f64::from(ds.compare(&dref, i).0)
            });
            out.row(&name, fmt, setting, bytes, "dssim", "1", Some(d), wt, ct);
            let cviews = views(cand, has_alpha);
            for s in &scales {
                let (cv, wt, ct) = timed(|| cviews.iter().map(|v| downscale(v, s.factor)).collect::<Vec<_>>());
                out.row(&name, fmt, setting, bytes, "downscale", s.label, None, wt, ct);
                let (v, wt, ct) = timed(|| s.fs2.iter().zip(&cv).map(|(r, c)| r.compare(&slice(c)).expect("fs2")).fold(f64::INFINITY, f64::min));
                out.row(&name, fmt, setting, bytes, "fs2", s.label, Some(v), wt, ct);
                let (v, wt, ct) = timed(|| {
                    s.ba.iter().zip(&cv).map(|(r, c)| r.compare(c.as_raw()).expect("ba")).fold((0.0f64, 0.0f64), |a, r| (a.0.max(r.score), a.1.max(r.pnorm_3)))
                });
                out.row(&name, fmt, setting, bytes, "ba", s.label, Some(v.0), wt, ct);
                out.row(&name, fmt, setting, bytes, "ba3", s.label, Some(v.1), 0.0, 0.0);
                if s.label == "1mp" {
                    let (v, wt, ct) = timed(|| {
                        s.refs.iter().zip(&cv).map(|(r, c)| image_compare::rgb_similarity_structure(&image_compare::Algorithm::MSSIMSimple, r, c).expect("ic ssim").score).fold(f64::INFINITY, f64::min)
                    });
                    out.row(&name, fmt, setting, bytes, "ic_ssim", s.label, Some(v), wt, ct);
                    let (v, wt, ct) = timed(|| s.refs.iter().zip(&cv).map(|(r, c)| image_compare::rgb_hybrid_compare(r, c).expect("ic hybrid").score).fold(f64::INFINITY, f64::min));
                    out.row(&name, fmt, setting, bytes, "ic_hybrid", s.label, Some(v), wt, ct);
                }
            }
            if s2rs {
                let (v, wt, ct) = timed(|| {
                    scales[0].refs.iter().zip(&cviews).map(|(r, c)| ssimulacra2::compute_frame_ssimulacra2(to_s2rs(r), to_s2rs(c)).expect("s2rs")).fold(f64::INFINITY, f64::min)
                });
                out.row(&name, fmt, setting, bytes, "s2rs", "1", Some(v), wt, ct);
            }
        };

        let tiny_img = image::load_from_memory(&tiny)?.to_rgba8();
        score(&mut out, "tinypng", tiny.len(), &tiny_img, false);

        if fmt == "jpeg" {
            let rgb = img.to_rgb8();
            for q in JPEG_GRID.step_by(3) {
                let setting = format!("q{q}");
                let (bytes, wt, ct) = timed(|| mozjpeg_encode(&rgb, w, h, q).expect("mozjpeg"));
                out.row(&name, fmt, &setting, bytes.len(), "encode", "1", None, wt, ct);
                let (dec, wt, ct) = timed(|| image::load_from_memory(&bytes).expect("decode").to_rgba8());
                out.row(&name, fmt, &setting, bytes.len(), "decode", "1", None, wt, ct);
                score(&mut out, &setting, bytes.len(), &dec, q == 66 && !big);
                println!("  {setting}: {} bytes", bytes.len());
            }
        } else {
            for &n in PNG_GRID {
                let setting = format!("{n}c");
                let ((pal, idx), wt, ct) = timed(|| quantizr_quantize(&rgba, w, h, n).expect("quantizr"));
                let px: Vec<RGBA8> = idx.iter().map(|&i| pal[i as usize]).collect();
                let dec = RgbaImage::from_raw(w, h, px.iter().flat_map(|c| [c.r, c.g, c.b, c.a]).collect()).expect("rgba");
                let (bytes, wt2, ct2) = timed(|| encode_indexed_oxipng(w, h, &pal, &idx).expect("png"));
                out.row(&name, fmt, &setting, bytes.len(), "quantize", "1", None, wt, ct);
                out.row(&name, fmt, &setting, bytes.len(), "oxipng", "1", None, wt2, ct2);
                score(&mut out, &setting, bytes.len(), &dec, n == 128);
                println!("  {setting}: {} bytes", bytes.len());
            }
        }
        // Write as we go, so a long run can be inspected or resumed by hand.
        fs::write(&out_csv, out.rows.join("\n") + "\n")?;
    }

    fs::write(&out_csv, out.rows.join("\n") + "\n")?;
    println!("wrote {}", out_csv.display());
    Ok(())
}

fn to_s2rs(img: &RgbImage) -> ssimulacra2::Rgb {
    let data = img.pixels().map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0]).collect();
    ssimulacra2::Rgb::new(data, img.width() as usize, img.height() as usize, ssimulacra2::TransferCharacteristic::SRGB, ssimulacra2::ColorPrimaries::BT709).expect("s2rs rgb")
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn quantizr_quantize(rgba: &RgbaImage, w: u32, h: u32, colors: u32) -> Option<(Vec<RGBA8>, Vec<u8>)> {
    let img = quantizr::Image::new(rgba.as_raw(), w as usize, h as usize).ok()?;
    let mut opts = quantizr::Options::default();
    opts.set_max_colors(colors as i32).ok()?;
    let mut res = quantizr::QuantizeResult::quantize(&img, &opts);
    res.set_dithering_level(1.0).ok()?;
    let mut idx = vec![0u8; (w * h) as usize];
    res.remap_image(&img, &mut idx).ok()?;
    let p = res.get_palette();
    let pal = p.entries[..p.count as usize].iter().map(|c| RGBA8::new(c.r, c.g, c.b, c.a)).collect();
    Some((pal, idx))
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

fn mozjpeg_encode(rgb: &RgbImage, w: u32, h: u32, q: u8) -> Option<Vec<u8>> {
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    comp.set_size(w as usize, h as usize);
    comp.set_quality(q as f32);
    comp.set_progressive_mode();
    let mut comp = comp.start_compress(Vec::new()).ok()?;
    comp.write_scanlines(&rgb.as_raw()[..]).ok()?;
    comp.finish().ok()
}

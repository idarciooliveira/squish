//! Phase 0 spike: candidate pipelines over bench/images/.
//!
//! PNG: quantette+oxipng, imagequant+oxipng, oxipng-only, plus the permissive RGBA
//! quantizers quantizr, exoquant (k-means) and NeuQuant (color_quant), each + oxipng.
//! JPEG: mozjpeg at quality 75/80/85, progressive.
//! TinyPNG's output for the same file, when present, is scored as the `tinypng` variant.
//! Every candidate is decoded again and compared to the original with dssim.
//! Results go to a CSV: file,variant,original_bytes,out_bytes,saved_pct,encode_ms,dssim,note.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::GenericImageView;
use rgb::FromSlice;

type Res<T> = Result<T, Box<dyn Error>>;

struct Candidate {
    variant: &'static str,
    bytes: Vec<u8>,
    ms: u128,
    note: String,
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).cloned().unwrap_or_else(|| "../images".into()));
    let out_csv = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "../results/spike.csv".into()));
    let tiny_dir = root.join("../../tinypng").canonicalize().unwrap_or(root.join("../tinypng"));

    let mut files: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.file_name().unwrap_or_default() != ".gitkeep")
        .collect();
    files.sort();
    println!("{} files in {}", files.len(), root.display());

    let dssim = dssim::Dssim::new();
    let mut rows: Vec<String> = vec![
        "file,format,variant,original_bytes,out_bytes,saved_pct,encode_ms,dssim,tinypng_bytes,note"
            .into(),
    ];

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let orig = fs::read(path)?;
        let fmt = detect(&orig).unwrap_or("other");
        println!("--- {name} ({fmt}, {} bytes)", orig.len());

        let dyn_img = match image::load_from_memory(&orig) {
            Ok(i) => i,
            Err(e) => {
                rows.push(csv(&[&name, fmt, "decode-failed", &n(orig.len()), "", "", "", "", &tiny(&tiny_dir, &name), &e.to_string()]));
                continue;
            }
        };
        let (w, h) = dyn_img.dimensions();
        let rgba = dyn_img.to_rgba8();
        let rgb = dyn_img.to_rgb8();
        let has_alpha = rgba.pixels().any(|p| p[3] != 255);

        let mut cands: Vec<Candidate> = vec![];
        if fmt == "png" {
            cands.push(png_oxipng_only(&orig));
            if has_alpha {
                cands.push(Candidate {
                    variant: "quantette+oxipng",
                    bytes: vec![],
                    ms: 0,
                    note: "skipped: image has alpha, quantette 0.6 is RGB-only".into(),
                });
            } else {
                cands.push(png_quantette_oxipng(&rgb, w, h));
            }
            cands.push(png_imagequant_oxipng(&rgba, w, h));
            cands.push(png_quantizr_oxipng(&rgba, w, h));
            cands.push(png_exoquant_oxipng(&rgba, w, h));
            cands.push(png_neuquant_oxipng(&rgba, w, h));
        } else if fmt == "jpeg" {
            for q in [75u8, 80, 85] {
                cands.push(jpeg_mozjpeg(&rgb, w, h, q));
            }
        }

        if let Ok(b) = fs::read(tiny_dir.join(&name)) {
            cands.push(Candidate { variant: "tinypng", bytes: b, ms: 0, note: String::new() });
        }

        let orig_dssim = dssim
            .create_image_rgba(rgba.as_raw().as_rgba(), w as usize, h as usize)
            .ok_or("dssim rejected original")?;

        for c in &cands {
            let (out_len, saved, dssim_s, note) = if c.bytes.is_empty() {
                ("".into(), "".into(), "".into(), c.note.clone())
            } else {
                let saved = 100.0 * (orig.len() as f64 - c.bytes.len() as f64) / orig.len() as f64;
                let ds = match image::load_from_memory(&c.bytes) {
                    Ok(d) => {
                        let r = d.to_rgba8();
                        let (dw, dh) = d.dimensions();
                        if dw != w || dh != h {
                            "dims-changed".into()
                        } else if let Some(img) =
                            dssim.create_image_rgba(r.as_raw().as_rgba(), w as usize, h as usize)
                        {
                            let (v, _) = dssim.compare(&orig_dssim, img);
                            format!("{:.5}", f64::from(v))
                        } else {
                            "dssim-rejected".into()
                        }
                    }
                    Err(e) => format!("undecodable: {e}"),
                };
                (n(c.bytes.len()), format!("{saved:.1}"), ds, c.note.clone())
            };
            println!(
                "  {:22} {:>9} -> {:>9} ({:>6}%) dssim={} {}",
                c.variant,
                orig.len(),
                if c.bytes.is_empty() { "-".into() } else { n(c.bytes.len()) },
                if c.bytes.is_empty() { "-".into() } else { saved.clone() },
                dssim_s,
                note
            );
            rows.push(csv(&[
                &name,
                fmt,
                c.variant,
                &n(orig.len()),
                &out_len,
                &saved,
                &c.ms.to_string(),
                &dssim_s,
                &tiny(&tiny_dir, &name),
                &note,
            ]));
        }
    }

    if let Some(p) = out_csv.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(&out_csv, rows.join("\n") + "\n")?;
    println!("wrote {}", out_csv.display());
    Ok(())
}

fn detect(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else {
        None
    }
}

fn n(v: usize) -> String {
    v.to_string()
}

fn tiny(dir: &Path, name: &str) -> String {
    fs::metadata(dir.join(name)).map(|m| m.len().to_string()).unwrap_or_default()
}

fn csv(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|f| {
            if f.contains([',', '"', '\n']) {
                format!("\"{}\"", f.replace('"', "\"\""))
            } else {
                f.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn encode_indexed_png(
    w: u32,
    h: u32,
    palette_rgb: &[[u8; 3]],
    palette_a: Option<&[u8]>,
    indices: &[u8],
) -> Res<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_palette(palette_rgb.concat());
        if let Some(a) = palette_a {
            if a.iter().any(|&v| v != 255) {
                enc.set_trns(a.to_vec());
            }
        }
        let mut wtr = enc.write_header()?;
        wtr.write_image_data(indices)?;
    }
    Ok(buf)
}

fn oxipng_preset2(png_bytes: &[u8]) -> Res<Vec<u8>> {
    let opts = oxipng::Options::from_preset(2);
    Ok(oxipng::optimize_from_memory(png_bytes, &opts)?)
}

fn png_oxipng_only(orig: &[u8]) -> Candidate {
    let t = Instant::now();
    match oxipng_preset2(orig) {
        Ok(b) => Candidate { variant: "oxipng-only", bytes: b, ms: t.elapsed().as_millis(), note: String::new() },
        Err(e) => Candidate { variant: "oxipng-only", bytes: vec![], ms: t.elapsed().as_millis(), note: format!("oxipng failed: {e}") },
    }
}

fn png_quantette_oxipng(rgb: &image::RgbImage, w: u32, h: u32) -> Candidate {
    let t = Instant::now();
    let fail = |e: String| Candidate {
        variant: "quantette+oxipng",
        bytes: vec![],
        ms: t.elapsed().as_millis(),
        note: e,
    };
    let imgref = match quantette::ImageRef::try_from(rgb) {
        Ok(i) => i,
        Err(e) => return fail(format!("imageref: {e:?}")),
    };
    let indexed = quantette::Pipeline::new().input_image(imgref).output_srgb8_indexed_image();
    let (pal, idx) = indexed.into_parts();
    if pal.is_empty() || pal.len() > 256 {
        return fail(format!("palette size {}", pal.len()));
    }
    let prgb: Vec<[u8; 3]> = pal.iter().map(|c| [c.red, c.green, c.blue]).collect();
    let png_bytes = match encode_indexed_png(w, h, &prgb, None, &idx) {
        Ok(b) => b,
        Err(e) => return fail(format!("png encode: {e}")),
    };
    match oxipng_preset2(&png_bytes) {
        Ok(b) => Candidate { variant: "quantette+oxipng", bytes: b, ms: t.elapsed().as_millis(), note: format!("{} colors", pal.len()) },
        Err(e) => Candidate { variant: "quantette+oxipng", bytes: png_bytes, ms: t.elapsed().as_millis(), note: format!("oxipng failed, raw indexed kept: {e}") },
    }
}

fn png_imagequant_oxipng(rgba: &image::RgbaImage, w: u32, h: u32) -> Candidate {
    let t = Instant::now();
    let fail = |e: String| Candidate {
        variant: "imagequant+oxipng",
        bytes: vec![],
        ms: t.elapsed().as_millis(),
        note: e,
    };
    let px: Vec<imagequant::RGBA> = rgba
        .pixels()
        .map(|p| imagequant::RGBA::new(p[0], p[1], p[2], p[3]))
        .collect();
    let mut liq = imagequant::new();
    if liq.set_speed(4).is_err() {
        return fail("set_speed".into());
    }
    if liq.set_quality(65, 80).is_err() {
        return fail("set_quality".into());
    }
    let mut img = match liq.new_image(&px[..], w as usize, h as usize, 0.0) {
        Ok(i) => i,
        Err(e) => return fail(format!("new_image: {e:?}")),
    };
    let mut res = match liq.quantize(&mut img) {
        Ok(r) => r,
        Err(e) => return fail(format!("quantize: {e:?}")),
    };
    let _ = res.set_dithering_level(1.0);
    let (pal, idx) = match res.remapped(&mut img) {
        Ok(v) => v,
        Err(e) => return fail(format!("remap: {e:?}")),
    };
    let prgb: Vec<[u8; 3]> = pal.iter().map(|c| [c.r, c.g, c.b]).collect();
    let pa: Vec<u8> = pal.iter().map(|c| c.a).collect();
    let png_bytes = match encode_indexed_png(w, h, &prgb, Some(&pa), &idx) {
        Ok(b) => b,
        Err(e) => return fail(format!("png encode: {e}")),
    };
    match oxipng_preset2(&png_bytes) {
        Ok(b) => Candidate {
            variant: "imagequant+oxipng",
            bytes: b,
            ms: t.elapsed().as_millis(),
            note: format!("{} colors, q{:?}", pal.len(), res.quantization_quality()),
        },
        Err(e) => Candidate { variant: "imagequant+oxipng", bytes: png_bytes, ms: t.elapsed().as_millis(), note: format!("oxipng failed, raw indexed kept: {e}") },
    }
}

/// Encodes an RGBA palette + indices as an indexed PNG, then runs oxipng.
fn indexed_rgba_oxipng(
    variant: &'static str,
    t: Instant,
    w: u32,
    h: u32,
    pal: &[[u8; 4]],
    idx: &[u8],
) -> Candidate {
    let prgb: Vec<[u8; 3]> = pal.iter().map(|c| [c[0], c[1], c[2]]).collect();
    let pa: Vec<u8> = pal.iter().map(|c| c[3]).collect();
    let png_bytes = match encode_indexed_png(w, h, &prgb, Some(&pa), idx) {
        Ok(b) => b,
        Err(e) => return Candidate { variant, bytes: vec![], ms: t.elapsed().as_millis(), note: format!("png encode: {e}") },
    };
    let note = format!("{} colors", pal.len());
    match oxipng_preset2(&png_bytes) {
        Ok(b) => Candidate { variant, bytes: b, ms: t.elapsed().as_millis(), note },
        Err(e) => Candidate { variant, bytes: png_bytes, ms: t.elapsed().as_millis(), note: format!("oxipng failed, raw indexed kept: {e}") },
    }
}

fn png_quantizr_oxipng(rgba: &image::RgbaImage, w: u32, h: u32) -> Candidate {
    let variant = "quantizr+oxipng";
    let t = Instant::now();
    let fail = |e: String| Candidate { variant, bytes: vec![], ms: t.elapsed().as_millis(), note: e };
    let img = match quantizr::Image::new(rgba.as_raw(), w as usize, h as usize) {
        Ok(i) => i,
        Err(e) => return fail(format!("image: {e:?}")),
    };
    let mut opts = quantizr::Options::default();
    if let Err(e) = opts.set_max_colors(256) {
        return fail(format!("max_colors: {e:?}"));
    }
    let mut res = quantizr::QuantizeResult::quantize(&img, &opts);
    let _ = res.set_dithering_level(1.0);
    let mut idx = vec![0u8; (w * h) as usize];
    if let Err(e) = res.remap_image(&img, &mut idx) {
        return fail(format!("remap: {e:?}"));
    }
    let p = res.get_palette();
    let pal: Vec<[u8; 4]> = p.entries[..p.count as usize].iter().map(|c| [c.r, c.g, c.b, c.a]).collect();
    indexed_rgba_oxipng(variant, t, w, h, &pal, &idx)
}

fn png_exoquant_oxipng(rgba: &image::RgbaImage, w: u32, h: u32) -> Candidate {
    use exoquant::{ditherer, optimizer, Color};
    let t = Instant::now();
    let px: Vec<Color> = rgba.pixels().map(|p| Color::new(p[0], p[1], p[2], p[3])).collect();
    let (pal, idx) = exoquant::convert_to_indexed(&px, w as usize, 256, &optimizer::KMeans, &ditherer::FloydSteinberg::new());
    let pal: Vec<[u8; 4]> = pal.iter().map(|c| [c.r, c.g, c.b, c.a]).collect();
    indexed_rgba_oxipng("exoquant+oxipng", t, w, h, &pal, &idx)
}

fn png_neuquant_oxipng(rgba: &image::RgbaImage, w: u32, h: u32) -> Candidate {
    let t = Instant::now();
    let nq = color_quant::NeuQuant::new(10, 256, rgba.as_raw());
    let idx: Vec<u8> = rgba.as_raw().chunks_exact(4).map(|p| nq.index_of(p) as u8).collect();
    let pal: Vec<[u8; 4]> = nq.color_map_rgba().chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    indexed_rgba_oxipng("neuquant+oxipng", t, w, h, &pal, &idx)
}

fn jpeg_mozjpeg(rgb: &image::RgbImage, w: u32, h: u32, q: u8) -> Candidate {
    let variant: &'static str = match q {
        75 => "mozjpeg-q75",
        80 => "mozjpeg-q80",
        _ => "mozjpeg-q85",
    };
    let t = Instant::now();
    let fail = |e: String| Candidate { variant, bytes: vec![], ms: t.elapsed().as_millis(), note: e };
    let mut comp = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    comp.set_size(w as usize, h as usize);
    comp.set_quality(q as f32);
    comp.set_progressive_mode();
    let mut comp = match comp.start_compress(Vec::new()) {
        Ok(c) => c,
        Err(e) => return fail(format!("start: {e}")),
    };
    if let Err(e) = comp.write_scanlines(&rgb.as_raw()[..]) {
        return fail(format!("scanlines: {e}"));
    }
    match comp.finish() {
        Ok(out) => Candidate { variant, bytes: out, ms: t.elapsed().as_millis(), note: String::new() },
        Err(e) => fail(format!("finish: {e}")),
    }
}

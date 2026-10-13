//! Runs squish-core over bench/images/ and checks NFR-1 (ADR-0025) and NFR-2 (ADR-0026).
//!
//! For each file with a TinyPNG counterpart it times the default search and a plain
//! fixed-setting encode, then scores both squish's and TinyPNG's output with dssim,
//! the referee, and with the shipped metric. Files run one at a time so CPU time is clean.
//!
//! WebP files are reported as `webp`, or `webp-lossless` for lossless sources, which
//! `--only webp` takes together. squish-core has no WebP codec yet, so until it does
//! every WebP file is listed under "skipped".
//!
//! usage: harness [--images DIR] [--tinypng DIR] [--out FILE] [--only png|jpeg|webp]
//!                [--effort fast|default|max] [--jpeg-target N] [--png-target N]
//!                [--webp-target N]

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dssim::Dssim;
use rgb::FromSlice;
use squish_core::{Effort, Format, Options, Quality, compress, detect, score};

type Res<T> = Result<T, Box<dyn Error>>;

/// dssim below this counts as equal, whatever TinyPNG scored (ADR-0025).
const FLOOR: f64 = 0.0002;

struct Args {
    images: PathBuf,
    tinypng: PathBuf,
    out: PathBuf,
    only: Option<String>,
    effort: Effort,
    jpeg_target: Option<f64>,
    png_target: Option<f64>,
    webp_target: Option<f64>,
}

fn parse_args() -> Res<Args> {
    let mut a = Args {
        images: "../images".into(),
        tinypng: "../tinypng".into(),
        out: "../results/phase1.csv".into(),
        only: None,
        effort: Effort::Default,
        jpeg_target: None,
        png_target: None,
        webp_target: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--images" => a.images = value()?.into(),
            "--tinypng" => a.tinypng = value()?.into(),
            "--out" => a.out = value()?.into(),
            "--only" => {
                let only = value()?;
                if !["png", "jpeg", "webp"].contains(&only.as_str()) {
                    return Err(format!("--only takes png, jpeg or webp, not {only}").into());
                }
                a.only = Some(only);
            }
            "--effort" => a.effort = value()?.parse()?,
            "--jpeg-target" => a.jpeg_target = Some(value()?.parse()?),
            "--png-target" => a.png_target = Some(value()?.parse()?),
            "--webp-target" => a.webp_target = Some(value()?.parse()?),
            other => return Err(format!("unknown flag {other}").into()),
        }
    }
    Ok(a)
}

fn cpu_ms() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let t = |v: libc::timeval| v.tv_sec as f64 * 1000.0 + v.tv_usec as f64 / 1000.0;
    t(u.ru_utime) + t(u.ru_stime)
}

/// Runs `f`, returning its result with wall and CPU milliseconds.
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64, f64) {
    let (wall, cpu) = (Instant::now(), cpu_ms());
    let value = f();
    (value, wall.elapsed().as_secs_f64() * 1000.0, cpu_ms() - cpu)
}

#[derive(Clone, Default)]
struct Row {
    file: String,
    format: String,
    width: u32,
    height: u32,
    original: usize,
    squish: usize,
    tinypng: usize,
    kept_original: bool,
    setting: Option<u32>,
    probes: u32,
    target: Option<f64>,
    fs2_squish: Option<f64>,
    fs2_tinypng: Option<f64>,
    dssim_squish: f64,
    dssim_tinypng: f64,
    wall_ms: f64,
    cpu_ms: f64,
    plain_cpu_ms: f64,
}

impl Row {
    /// Our dssim over TinyPNG's, with TinyPNG's floored (ADR-0025).
    fn ratio(&self) -> f64 {
        self.dssim_squish / self.dssim_tinypng.max(FLOOR)
    }
    fn cpu_ratio(&self) -> f64 {
        self.cpu_ms / self.plain_cpu_ms.max(1.0)
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn opt(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.3}")).unwrap_or_default()
}

fn write_csv(path: &Path, rows: &[Row]) -> Res<()> {
    let mut out = String::from(
        "file,format,width,height,original_bytes,squish_bytes,tinypng_bytes,kept_original,setting,probes,target,\
         fs2_squish,fs2_tinypng,dssim_squish,dssim_tinypng,dssim_ratio,wall_ms,cpu_ms,plain_cpu_ms,cpu_ratio\n",
    );
    for r in rows {
        out += &format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{:.6},{:.6},{:.3},{:.1},{:.1},{:.1},{:.2}\n",
            csv_field(&r.file),
            r.format,
            r.width,
            r.height,
            r.original,
            r.squish,
            r.tinypng,
            r.kept_original,
            r.setting.map(|s| s.to_string()).unwrap_or_default(),
            r.probes,
            opt(r.target),
            opt(r.fs2_squish),
            opt(r.fs2_tinypng),
            r.dssim_squish,
            r.dssim_tinypng,
            r.ratio(),
            r.wall_ms,
            r.cpu_ms,
            r.plain_cpu_ms,
            r.cpu_ratio(),
        );
    }
    fs::write(path, out)?;
    Ok(())
}

/// True when a WebP file has a lossless (`VP8L`) bitstream. Walks the RIFF chunks.
fn webp_is_lossless(bytes: &[u8]) -> bool {
    let mut at = 12;
    while let Some(header) = bytes.get(at..at + 8) {
        if &header[..4] == b"VP8L" {
            return true;
        }
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        at += 8 + size + (size & 1);
    }
    false
}

fn decode_rgba(bytes: &[u8]) -> Res<image::RgbaImage> {
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    match values.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => values[n / 2],
        n => (values[n / 2 - 1] + values[n / 2]) / 2.0,
    }
}

fn main() -> Res<()> {
    let args = parse_args()?;
    let mut files: Vec<PathBuf> = fs::read_dir(&args.images)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n != ".gitkeep"))
        .collect();
    files.sort();

    let dssim = Dssim::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let original = fs::read(path)?;
        let Some(format) = detect(&original) else {
            skipped.push(format!("{name}: not an image squish knows"));
            continue;
        };
        let format_name = match format {
            Format::WebP if webp_is_lossless(&original) => "webp-lossless",
            other => other.name(),
        };
        if !matches!(format, Format::Png | Format::Jpeg | Format::WebP)
            || args.only.as_deref().is_some_and(|o| !format_name.starts_with(o))
        {
            continue;
        }
        let Ok(tiny) = fs::read(args.tinypng.join(&name)) else {
            skipped.push(format!("{name}: no TinyPNG output"));
            continue;
        };

        let mut options = Options { effort: args.effort, ..Options::default() };
        options.target = match format {
            Format::Jpeg => args.jpeg_target,
            Format::WebP => args.webp_target,
            _ => args.png_target,
        };

        // One plain encode: the same pipeline at a fixed setting, with no search.
        let plain = Options {
            quality: Quality::Fixed(match format {
                Format::Jpeg => 66,
                Format::WebP => 71,
                _ => 100,
            }),
            effort: Effort::Default,
            ..Options::default()
        };
        let (plain_result, _, plain_cpu) = timed(|| compress(&original, &plain));
        let (result, wall, cpu) = timed(|| compress(&original, &options));
        let out = match (result, plain_result) {
            (Ok(out), Ok(_)) => out,
            (Err(e), _) | (_, Err(e)) => {
                skipped.push(format!("{name}: {e}"));
                println!("--- {name}: skipped, {e}");
                continue;
            }
        };

        let ref_img = decode_rgba(&original)?;
        let (w, h) = ref_img.dimensions();
        let reference = dssim
            .create_image_rgba(ref_img.as_raw().as_rgba(), w as usize, h as usize)
            .ok_or("dssim rejected the original")?;
        let dssim_of = |bytes: &[u8]| -> Res<f64> {
            let img = decode_rgba(bytes)?;
            let candidate = dssim
                .create_image_rgba(img.as_raw().as_rgba(), w as usize, h as usize)
                .ok_or("dssim rejected the candidate")?;
            Ok(f64::from(dssim.compare(&reference, candidate).0))
        };
        let dssim_squish = dssim_of(&out.bytes)?;
        let dssim_tinypng = dssim_of(&tiny)?;

        let row = Row {
            file: name.clone(),
            format: format_name.to_string(),
            width: w,
            height: h,
            original: original.len(),
            squish: out.bytes.len(),
            tinypng: tiny.len(),
            kept_original: out.kept_original,
            setting: out.report.setting,
            probes: out.report.probes,
            target: out.report.target,
            fs2_squish: score(&original, &out.bytes).ok(),
            fs2_tinypng: score(&original, &tiny).ok(),
            dssim_squish,
            dssim_tinypng,
            wall_ms: wall,
            cpu_ms: cpu,
            plain_cpu_ms: plain_cpu,
        };
        println!(
            "{:<42} {:>5} {:>9} -> {:>9} (tinypng {:>9})  set {:>4}  probes {}  dssim x{:.2}  cpu {:.0} ms = {:.1}x plain",
            name,
            format_name,
            row.original,
            row.squish,
            row.tinypng,
            row.setting.map(|s| s.to_string()).unwrap_or_else(|| "-".into()),
            row.probes,
            row.ratio(),
            row.cpu_ms,
            row.cpu_ratio(),
        );
        rows.push(row);
        write_csv(&args.out, &rows)?;
    }

    write_csv(&args.out, &rows)?;
    summarize(&rows, &skipped, &args);
    println!("\nwrote {}", args.out.display());
    Ok(())
}

fn summarize(rows: &[Row], skipped: &[String], args: &Args) {
    println!("\n=== summary (effort {}) ===", args.effort);
    for format in ["jpeg", "png", "webp", "webp-lossless"] {
        let set: Vec<&Row> = rows.iter().filter(|r| r.format == format).collect();
        if set.is_empty() {
            continue;
        }
        let ours: usize = set.iter().map(|r| r.squish).sum();
        let theirs: usize = set.iter().map(|r| r.tinypng).sum();
        let bytes_pct = (ours as f64 / theirs as f64 - 1.0) * 100.0;
        let mut ratios: Vec<f64> = set.iter().map(|r| r.ratio()).collect();
        let med = median(&mut ratios);
        // ADR-0025 point 3: for PNG, files already at 256 colors don't count toward (c).
        let counted: Vec<&&Row> = set.iter().filter(|r| !(format == "png" && r.setting == Some(256))).collect();
        let worst = counted.iter().max_by(|a, b| a.ratio().total_cmp(&b.ratio()));
        let over: Vec<&&&Row> = counted.iter().filter(|r| r.ratio() > 2.0).collect();
        let mut cpu_ratios: Vec<f64> = set.iter().filter(|r| r.probes > 0).map(|r| r.cpu_ratio()).collect();
        let cpu_med = median(&mut cpu_ratios);
        let slowest = set.iter().max_by(|a, b| a.cpu_ms.total_cmp(&b.cpu_ms)).unwrap();
        let over_budget = set.iter().filter(|r| r.cpu_ms > 10_000.0).count();
        let kept = set.iter().filter(|r| r.kept_original).count();
        let missed = set.iter().filter(|r| r.fs2_squish.zip(r.target).is_some_and(|(s, t)| s < t)).count();

        let verdict = |ok: bool| if ok { "PASS" } else { "FAIL" };
        println!("\n{format}: {} files, {kept} kept as is, {missed} missed their target", set.len());
        println!("  NFR-1 (a) total bytes {bytes_pct:+.1}% vs TinyPNG (limit +5.0%)   {}", verdict(bytes_pct <= 5.0));
        println!("  NFR-1 (b) median dssim ratio {med:.2} (limit 1.00)   {}", verdict(med <= 1.0));
        match worst {
            Some(w) => println!(
                "  NFR-1 (c) worst dssim ratio {:.2} on {} (limit 2.00, {} counted)   {}",
                w.ratio(),
                w.file,
                counted.len(),
                verdict(over.is_empty())
            ),
            None => println!("  NFR-1 (c) no files counted"),
        }
        for r in &over {
            println!("            over 2.0: {} x{:.2} setting {:?}", r.file, r.ratio(), r.setting);
        }
        println!("  NFR-2 median search cost {cpu_med:.1}x one plain encode (limit 4.0x)   {}", verdict(cpu_med <= 4.0));
        println!(
            "  NFR-2 slowest {:.1} s CPU on {} (limit 10 s), {over_budget} files over   {}",
            slowest.cpu_ms / 1000.0,
            slowest.file,
            verdict(over_budget == 0)
        );
    }
    if !skipped.is_empty() {
        println!("\nskipped {}:", skipped.len());
        for s in skipped {
            println!("  {s}");
        }
    }
}

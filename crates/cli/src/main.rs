//! squish: compress PNG and JPEG files from the command line (ADR-0019).

mod inputs;
mod plan;
mod run;
mod summary;
mod write;

use std::io::{self, IsTerminal, Read, Write as _};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use clap::Parser;
use squish_core::{Effort, MetaPolicy, Options, Quality};

use crate::plan::Mode;
use crate::run::{Job, Outcome, Status, compress_bytes};
use crate::summary::Summary;

const USAGE: u8 = 2;
const INTERRUPTED: u8 = 130;

#[derive(Parser, Debug)]
#[command(name = "squish", version, about = "Compress PNG and JPEG images, keeping their format")]
struct Cli {
    /// Files, folders, globs, or - to read one image from stdin and write it to stdout
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Mirror inputs into DIR with their original names
    #[arg(short, long, value_name = "DIR", help_heading = "Output (default writes name.min.ext next to the input)")]
    out_dir: Option<PathBuf>,

    /// Overwrite the inputs (asks first unless --yes)
    #[arg(long, conflicts_with = "out_dir", help_heading = "Output (default writes name.min.ext next to the input)")]
    in_place: bool,

    /// Descend into folders
    #[arg(short, long, help_heading = "Inputs")]
    recursive: bool,

    /// auto | lossless | 0-100
    #[arg(long, value_name = "Q", default_value = "auto", help_heading = "Compression")]
    quality: Quality,

    /// Same as --quality lossless
    #[arg(long, conflicts_with = "quality", help_heading = "Compression")]
    lossless: bool,

    /// fast | default | max
    #[arg(short, long, value_name = "E", default_value = "default", help_heading = "Compression")]
    effort: Effort,

    /// strip-keep-icc | strip-all | keep-all
    #[arg(long, value_name = "M", default_value = "strip-keep-icc", help_heading = "Compression")]
    metadata: MetaPolicy,

    /// Refuse images above this many megapixels
    #[arg(long, value_name = "MP", default_value = "50", value_parser = parse_megapixels, help_heading = "Compression")]
    max_pixels: u64,

    /// Parallel jobs [default: core count]
    #[arg(short, long, value_name = "N", help_heading = "Run")]
    jobs: Option<NonZeroUsize>,

    /// Don't ask before --in-place
    #[arg(short, long, help_heading = "Run")]
    yes: bool,

    /// No progress bar, no summary; errors still print
    #[arg(short, long, help_heading = "Run")]
    quiet: bool,
}

fn parse_megapixels(s: &str) -> Result<u64, String> {
    match s.trim().parse::<f64>() {
        Ok(mp) if mp.is_finite() && mp > 0.0 => Ok((mp * 1e6).round() as u64),
        _ => Err(format!("invalid megapixels {s:?}, expected a positive number such as 50 or 0.5")),
    }
}

impl Cli {
    fn options(&self) -> Options {
        Options {
            quality: if self.lossless { Quality::Lossless } else { self.quality },
            effort: self.effort,
            metadata: self.metadata,
            max_pixels: self.max_pixels,
            ..Options::default()
        }
    }
}

fn main() -> ExitCode {
    // `serve` is reserved (ADR-0019). The web version arrives in Phase 3.
    if std::env::args_os().nth(1).is_some_and(|a| a == "serve") {
        eprintln!("squish: `squish serve` is not built yet. To compress a file named serve, write ./serve");
        return ExitCode::from(USAGE);
    }
    let cli = Cli::parse();
    if cli.paths.iter().any(|p| p == Path::new("-")) {
        return stdin_mode(&cli);
    }
    files_mode(&cli)
}

fn usage_error(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("squish: {message}");
    ExitCode::from(USAGE)
}

/// `squish -`: one image from stdin to stdout, the summary on stderr so the pipe stays clean.
fn stdin_mode(cli: &Cli) -> ExitCode {
    if cli.paths.len() > 1 || cli.out_dir.is_some() || cli.in_place {
        return usage_error("- can't be mixed with other paths, --out-dir or --in-place");
    }
    if io::stdout().is_terminal() {
        return usage_error("refusing to write image bytes to a terminal; redirect stdout to a file");
    }
    let started = Instant::now();
    let mut bytes = Vec::new();
    if let Err(e) = io::stdin().read_to_end(&mut bytes) {
        eprintln!("squish: could not read stdin: {e}");
        return ExitCode::FAILURE;
    }
    let (out, status) = match compress_bytes(&bytes, &cli.options()) {
        Outcome::Done(out) if out.kept_original => (out.bytes, Status::AlreadyOptimized),
        Outcome::Done(out) => (out.bytes, Status::Compressed),
        // Pass the input through, so `squish - < a.gif > b.gif` still produces the file.
        Outcome::Unsupported(e) => {
            eprintln!("squish: -: {e}, passing it through unchanged");
            (bytes.clone(), Status::Unsupported)
        }
        Outcome::Failed(_, message) => {
            eprintln!("squish: -: {message}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = io::stdout().lock().write_all(&out).and_then(|()| io::stdout().flush()) {
        eprintln!("squish: could not write stdout: {e}");
        return ExitCode::FAILURE;
    }
    if !cli.quiet && status != Status::Unsupported {
        eprintln!(
            "  -   {} -> {}   ({})   {:.1} s",
            summary::bytes(bytes.len() as u64),
            summary::bytes(out.len() as u64),
            summary::percent(bytes.len() as u64, out.len() as u64),
            started.elapsed().as_secs_f64()
        );
    }
    ExitCode::SUCCESS
}

fn files_mode(cli: &Cli) -> ExitCode {
    let started = Instant::now();
    let mode = match (&cli.out_dir, cli.in_place) {
        (Some(dir), _) => Mode::OutDir(dir.clone()),
        (None, true) => Mode::InPlace,
        (None, false) => Mode::Suffix,
    };
    let collected = match inputs::collect(&cli.paths, cli.recursive, cli.out_dir.as_deref()) {
        Ok(c) => c,
        Err(e) => return usage_error(e),
    };
    let outputs = match plan::outputs(&collected.inputs, &mode) {
        Ok(o) => o,
        Err(e) => return usage_error(e),
    };
    if mode == Mode::InPlace && !cli.yes && !collected.inputs.is_empty() {
        match confirm_in_place(collected.inputs.len()) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("squish: nothing changed");
                return ExitCode::SUCCESS;
            }
            Err(e) => return usage_error(e),
        }
    }

    // First Ctrl-C: stop starting files, finish the running ones. Second: exit at once.
    // Every write is a rename, so both leave no partial files.
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    let _ = ctrlc::set_handler(move || {
        if flag.swap(true, Ordering::SeqCst) {
            std::process::exit(i32::from(INTERRUPTED));
        }
    });

    let jobs = cli.jobs.or_else(|| std::thread::available_parallelism().ok()).map_or(1, NonZeroUsize::get);
    let options = cli.options();
    let results = run::run(&Job {
        inputs: &collected.inputs,
        outputs: &outputs,
        mode: &mode,
        options: &options,
        jobs,
        quiet: cli.quiet,
        interrupted: &interrupted,
    });

    let was_interrupted = interrupted.load(Ordering::SeqCst);
    if !cli.quiet {
        let text = Summary {
            results: &results,
            problems: &collected.problems,
            not_started: collected.inputs.len() - results.len(),
            elapsed: started.elapsed(),
            jobs,
        }
        .render();
        print!("{text}");
    } else {
        for (path, message) in &collected.problems {
            eprintln!("squish: {}: {message}", path.display());
        }
    }

    if was_interrupted {
        ExitCode::from(INTERRUPTED)
    } else if !collected.problems.is_empty() || results.iter().any(|r| r.status == Status::Failed) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Asks on the terminal. Without one there is nobody to ask, which is a usage error.
fn confirm_in_place(count: usize) -> Result<bool, &'static str> {
    if !io::stdin().is_terminal() {
        return Err("--in-place needs --yes when stdin is not a terminal");
    }
    let noun = if count == 1 { "file" } else { "files" };
    eprint!("squish: overwrite {count} {noun} in place? [y/N] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(|_| "could not read the answer")?;
    Ok(matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn megapixels_parse() {
        assert_eq!(parse_megapixels("50"), Ok(50_000_000));
        assert_eq!(parse_megapixels("0.5"), Ok(500_000));
        assert!(parse_megapixels("0").is_err());
        assert!(parse_megapixels("-3").is_err());
        assert!(parse_megapixels("nan").is_err());
    }

    #[test]
    fn lossless_flag_sets_quality() {
        let cli = Cli::parse_from(["squish", "--lossless", "a.png"]);
        assert_eq!(cli.options().quality, Quality::Lossless);
        assert!(Cli::try_parse_from(["squish", "--lossless", "--quality", "80", "a.png"]).is_err());
        assert!(Cli::try_parse_from(["squish", "-o", "out", "--in-place", "a.png"]).is_err());
    }
}

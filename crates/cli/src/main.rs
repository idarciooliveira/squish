//! squish: compress PNG and JPEG files from the command line (ADR-0019).

mod archive;
mod inputs;
mod plan;
mod run;
mod serve;
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

use crate::archive::Archive;
use crate::plan::Mode;
use crate::run::{Job, Outcome, Status, compress_bytes};
use crate::summary::Summary;

pub(crate) const USAGE: u8 = 2;
const INTERRUPTED: u8 = 130;
const OUTPUT: &str = "Output (default writes name.min.ext next to the input)";

#[derive(Parser, Debug)]
#[command(
    name = "squish",
    version,
    about = "Compress PNG and JPEG images, keeping their format",
    after_help = "Run `squish serve --help` for the web UI."
)]
struct Cli {
    /// Files, folders, globs, or - to read one image from stdin and write it to stdout
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Mirror inputs into DIR with their original names
    #[arg(short, long, value_name = "DIR", help_heading = OUTPUT)]
    out_dir: Option<PathBuf>,

    /// Overwrite the inputs (asks first unless --yes)
    #[arg(long, conflicts_with = "out_dir", help_heading = OUTPUT)]
    in_place: bool,

    /// Write every output into one STORE zip
    #[arg(long, value_name = "FILE", conflicts_with_all = ["out_dir", "in_place"], help_heading = OUTPUT)]
    zip: Option<PathBuf>,

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

    /// Compress in memory, write nothing, report savings
    #[arg(short = 'n', long, help_heading = "Run")]
    dry_run: bool,

    /// Print a JSON report on stdout instead of text
    #[arg(long, help_heading = "Run")]
    json: bool,

    /// Don't ask before --in-place
    #[arg(short, long, help_heading = "Run")]
    yes: bool,

    /// No progress bar, no summary; errors still print
    #[arg(short, long, help_heading = "Run")]
    quiet: bool,
}

pub(crate) fn parse_megapixels(s: &str) -> Result<u64, String> {
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
    // `serve` is reserved (ADR-0019). To compress a file named serve, write ./serve.
    if std::env::args_os().nth(1).is_some_and(|a| a == "serve") {
        return serve::main(std::env::args_os().skip(1));
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
    if cli.paths.len() > 1 || cli.out_dir.is_some() || cli.in_place || cli.zip.is_some() || cli.json {
        return usage_error("- can't be mixed with other paths, --out-dir, --in-place, --zip or --json");
    }
    if !cli.dry_run && io::stdout().is_terminal() {
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
    if !cli.dry_run
        && let Err(e) = io::stdout().lock().write_all(&out).and_then(|()| io::stdout().flush())
    {
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
    let mode = match (&cli.out_dir, cli.in_place, &cli.zip) {
        (Some(dir), _, _) => Mode::OutDir(dir.clone()),
        (None, true, _) => Mode::InPlace,
        (None, false, Some(zip)) => Mode::Zip(zip.clone()),
        (None, false, None) => Mode::Suffix,
    };
    let collected = match inputs::collect(&cli.paths, cli.recursive, cli.out_dir.as_deref()) {
        Ok(c) => c,
        Err(e) => return usage_error(e),
    };
    let outputs = match plan::outputs(&collected.inputs, &mode) {
        Ok(o) => o,
        Err(e) => return usage_error(e),
    };
    if mode == Mode::InPlace && !cli.yes && !cli.dry_run && !collected.inputs.is_empty() {
        match confirm_in_place(collected.inputs.len()) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("squish: nothing changed");
                return ExitCode::SUCCESS;
            }
            Err(e) => return usage_error(e),
        }
    }

    let archive = match &mode {
        Mode::Zip(path) if !cli.dry_run => match Archive::create(path) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("squish: could not create {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        },
        _ => None,
    };

    // First Ctrl-C: stop starting files, finish the running ones. Second: exit at once.
    // Every write is a rename, so both leave no partial files. A zip keeps the entries
    // that finished, so it matches the summary.
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
        archive: archive.as_ref(),
        dry_run: cli.dry_run,
        options: &options,
        jobs,
        quiet: cli.quiet || cli.json,
        interrupted: &interrupted,
    });

    // A zip that can't be finished loses every entry, so the run failed whatever the files did.
    let mut zip_failed = false;
    if let (Some(archive), Mode::Zip(path)) = (archive, &mode)
        && let Err(e) = archive.finish()
    {
        eprintln!("squish: could not write {}: {e}", path.display());
        zip_failed = true;
    }

    let was_interrupted = interrupted.load(Ordering::SeqCst);
    let summary = Summary {
        results: &results,
        problems: &collected.problems,
        not_started: collected.inputs.len() - results.len(),
        elapsed: started.elapsed(),
        jobs,
        dry_run: cli.dry_run,
    };
    if cli.json {
        print!("{}", summary.json());
    } else if !cli.quiet {
        print!("{}", summary.render());
    } else {
        for (path, message) in &collected.problems {
            eprintln!("squish: {}: {message}", path.display());
        }
    }

    if was_interrupted {
        ExitCode::from(INTERRUPTED)
    } else if zip_failed || !collected.problems.is_empty() || results.iter().any(|r| r.status == Status::Failed) {
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

    #[test]
    fn zip_conflicts_with_the_other_output_modes() {
        assert!(Cli::try_parse_from(["squish", "--zip", "a.zip", "-o", "out", "a.png"]).is_err());
        assert!(Cli::try_parse_from(["squish", "--zip", "a.zip", "--in-place", "a.png"]).is_err());
        let cli = Cli::parse_from(["squish", "--zip", "a.zip", "-n", "--json", "a.png"]);
        assert!(cli.dry_run && cli.json && cli.zip.is_some());
    }
}

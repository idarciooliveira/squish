//! Compresses the planned files in parallel and records one outcome per file.

use std::io::IsTerminal;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use rayon::prelude::*;
use squish_core::{Error, Format, Options, compress};

use crate::inputs::Input;
use crate::plan::Mode;
use crate::write::atomic_write;

/// The four outcomes from ADR-0017.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Compressed,
    AlreadyOptimized,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub input: PathBuf,
    pub status: Status,
    pub format: Option<Format>,
    pub original_bytes: u64,
    pub output_bytes: u64,
    /// Error code (ADR-0017, plus the CLI's own I/O codes) and message.
    pub error: Option<(&'static str, String)>,
}

impl FileResult {
    fn failed(input: &Path, code: &'static str, message: String) -> Self {
        FileResult {
            input: input.to_path_buf(),
            status: Status::Failed,
            format: None,
            original_bytes: 0,
            output_bytes: 0,
            error: Some((code, message)),
        }
    }
}

/// What came back from compressing one buffer, before anything was written.
pub enum Outcome {
    Done(squish_core::Output),
    Unsupported(Error),
    Failed(&'static str, String),
}

/// Runs `compress` with panics caught, so one bad file can't take down the batch (ADR-0017).
pub fn compress_bytes(bytes: &[u8], options: &Options) -> Outcome {
    match catch_unwind(AssertUnwindSafe(|| compress(bytes, options))) {
        Ok(Ok(output)) => Outcome::Done(output),
        Ok(Err(e @ (Error::Unsupported(_) | Error::Animated(_)))) => Outcome::Unsupported(e),
        Ok(Err(e)) => Outcome::Failed(e.code(), e.to_string()),
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            Outcome::Failed("internal_error", format!("squish crashed on this file: {message}"))
        }
    }
}

pub struct Job<'a> {
    pub inputs: &'a [Input],
    pub outputs: &'a [PathBuf],
    pub mode: &'a Mode,
    pub options: &'a Options,
    pub jobs: usize,
    pub quiet: bool,
    pub interrupted: &'a AtomicBool,
}

/// Results in input order. Files not started because of Ctrl-C are left out.
pub fn run(job: &Job) -> Vec<FileResult> {
    let bar = progress_bar(job.inputs.len() as u64, job.quiet);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(job.jobs).build().expect("thread pool");
    let results: Vec<FileResult> = pool.install(|| {
        job.inputs
            .par_iter()
            .zip(job.outputs.par_iter())
            .filter_map(|(input, output)| {
                if job.interrupted.load(Ordering::SeqCst) {
                    return None;
                }
                let result = process(&input.path, output, job.mode, job.options);
                if let (Status::Failed, Some((_, message))) = (result.status, &result.error) {
                    bar.suspend(|| eprintln!("squish: {}: {message}", input.path.display()));
                }
                bar.inc(1);
                Some(result)
            })
            .collect()
    });
    bar.finish_and_clear();
    results
}

fn progress_bar(len: u64, quiet: bool) -> ProgressBar {
    if quiet || !std::io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::with_draw_target(Some(len), ProgressDrawTarget::stderr());
    bar.set_style(
        ProgressStyle::with_template("{bar:30.cyan/blue} {pos}/{len} {elapsed_precise} eta {eta}")
            .expect("valid template")
            .progress_chars("=> "),
    );
    bar
}

fn process(input: &Path, output: &Path, mode: &Mode, options: &Options) -> FileResult {
    let bytes = match std::fs::read(input) {
        Ok(b) => b,
        Err(e) => return FileResult::failed(input, "read_failed", e.to_string()),
    };
    let original_bytes = bytes.len() as u64;
    let out = match compress_bytes(&bytes, options) {
        Outcome::Done(out) => out,
        Outcome::Unsupported(e) => {
            let format = match e {
                Error::Unsupported(f) => f,
                Error::Animated(f) => Some(f),
                _ => None,
            };
            return FileResult {
                input: input.to_path_buf(),
                status: Status::Unsupported,
                format,
                original_bytes,
                output_bytes: 0,
                error: Some((e.code(), e.to_string())),
            };
        }
        Outcome::Failed(code, message) => return FileResult::failed(input, code, message),
    };

    // An already-optimized file still gets its copy, except in place, where there's nothing to do.
    let in_place = *mode == Mode::InPlace;
    if !(in_place && out.kept_original) {
        let permissions_from = in_place.then_some(input);
        if let Err(e) = atomic_write(output, &out.bytes, permissions_from) {
            let message = format!("could not write {}: {e}", output.display());
            return FileResult::failed(input, "write_failed", message);
        }
    }
    FileResult {
        input: input.to_path_buf(),
        status: if out.kept_original { Status::AlreadyOptimized } else { Status::Compressed },
        format: Some(out.format),
        original_bytes,
        output_bytes: out.bytes.len() as u64,
        error: None,
    }
}

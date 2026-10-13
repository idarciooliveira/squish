//! The end-of-run summary (PRD FR-11), as text or as the JSON report (ADR-0019).

use std::fmt::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::run::{FileResult, Status};

pub struct Summary<'a> {
    pub results: &'a [FileResult],
    /// Folders the walk could not read. They count as failed.
    pub problems: &'a [(PathBuf, String)],
    /// Files never started because of Ctrl-C.
    pub not_started: usize,
    pub elapsed: Duration,
    pub jobs: usize,
    pub dry_run: bool,
}

impl Summary<'_> {
    /// Bytes before and after, over the files that produced an output.
    fn totals(&self) -> (u64, u64) {
        let done = |r: &&FileResult| matches!(r.status, Status::Compressed | Status::AlreadyOptimized);
        let before = self.results.iter().filter(done).map(|r| r.original_bytes).sum();
        let after = self.results.iter().filter(done).map(|r| r.output_bytes).sum();
        (before, after)
    }

    pub fn render(&self) -> String {
        let (before, after) = self.totals();
        let files = self.results.len() + self.problems.len();
        let threads = if self.jobs == 1 { "thread" } else { "threads" };

        let mut s = String::new();
        let _ = writeln!(
            s,
            "  {files} {}   {} -> {}   ({})   {:.1} s on {} {threads}",
            if files == 1 { "file" } else { "files" },
            bytes(before),
            bytes(after),
            percent(before, after),
            self.elapsed.as_secs_f64(),
            self.jobs,
        );

        let with = |status| self.results.iter().filter(move |r| r.status == status);
        let kept: Vec<String> = with(Status::AlreadyOptimized).map(|r| r.input.display().to_string()).collect();
        list(&mut s, "already optimized, kept as is", &kept);
        let unsupported: Vec<String> = with(Status::Unsupported)
            .map(|r| match r.format {
                Some(format) => format!("{} ({format})", r.input.display()),
                None => r.input.display().to_string(),
            })
            .collect();
        list(&mut s, "unsupported", &unsupported);
        let failed: Vec<String> = with(Status::Failed)
            .map(|r| (r.input.display().to_string(), r.error.as_ref().map(|e| e.1.as_str()).unwrap_or_default()))
            .chain(self.problems.iter().map(|(p, e)| (p.display().to_string(), e.as_str())))
            .map(|(path, message)| format!("{path}: {message}"))
            .collect();
        list(&mut s, "failed", &failed);
        if self.not_started > 0 {
            let _ = writeln!(s, "  interrupted, {} not started", plural(self.not_started, "file"));
        }
        if self.dry_run {
            let _ = writeln!(s, "  dry run, nothing written");
        }
        s
    }

    /// The `--json` report: one document, so `jq` reads it directly.
    pub fn json(&self) -> String {
        let (original_bytes, output_bytes) = self.totals();
        let count = |status| self.results.iter().filter(|r| r.status == status).count();
        let files: Vec<JsonFile> = self
            .results
            .iter()
            .map(|r| JsonFile {
                input: r.input.display().to_string(),
                output: r.output.as_ref().map(|p| p.display().to_string()),
                format: r.format.map(|f| f.name()),
                status: r.status.name(),
                original_bytes: r.original_bytes,
                output_bytes: r.output_bytes,
                elapsed_ms: millis(r.elapsed),
                error: r.error.as_ref().map(|e| e.0),
                message: r.error.as_ref().map(|e| e.1.as_str()),
            })
            .chain(self.problems.iter().map(|(path, message)| JsonFile {
                input: path.display().to_string(),
                output: None,
                format: None,
                status: Status::Failed.name(),
                original_bytes: 0,
                output_bytes: 0,
                elapsed_ms: 0,
                error: Some("read_failed"),
                message: Some(message),
            }))
            .collect();
        let report = JsonReport {
            version: env!("CARGO_PKG_VERSION"),
            summary: JsonSummary {
                files: files.len(),
                compressed: count(Status::Compressed),
                already_optimized: count(Status::AlreadyOptimized),
                unsupported: count(Status::Unsupported),
                failed: count(Status::Failed) + self.problems.len(),
                not_started: self.not_started,
                original_bytes,
                output_bytes,
                elapsed_ms: millis(self.elapsed),
                jobs: self.jobs,
                dry_run: self.dry_run,
            },
            files,
        };
        let mut text = serde_json::to_string_pretty(&report).expect("the report serializes");
        text.push('\n');
        text
    }
}

#[derive(Serialize)]
struct JsonReport<'a> {
    version: &'static str,
    summary: JsonSummary,
    files: Vec<JsonFile<'a>>,
}

#[derive(Serialize)]
struct JsonSummary {
    files: usize,
    compressed: usize,
    already_optimized: usize,
    unsupported: usize,
    failed: usize,
    not_started: usize,
    original_bytes: u64,
    output_bytes: u64,
    elapsed_ms: u64,
    jobs: usize,
    dry_run: bool,
}

#[derive(Serialize)]
struct JsonFile<'a> {
    input: String,
    output: Option<String>,
    format: Option<&'static str>,
    status: &'static str,
    original_bytes: u64,
    output_bytes: u64,
    elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'a str>,
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// One line for a single entry, a header and an indented list for more.
fn list(s: &mut String, label: &str, entries: &[String]) {
    match entries {
        [] => {}
        [one] => {
            let _ = writeln!(s, "  1 {label}: {one}");
        }
        many => {
            let _ = writeln!(s, "  {} {label}:", many.len());
            for entry in many {
                let _ = writeln!(s, "    {entry}");
            }
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

/// Decimal units, like the PRD's examples.
pub fn bytes(n: u64) -> String {
    let n = n as f64;
    match n {
        n if n < 1e3 => format!("{n} B"),
        n if n < 1e6 => format!("{:.1} KB", n / 1e3),
        n if n < 1e9 => format!("{:.1} MB", n / 1e6),
        n => format!("{:.1} GB", n / 1e9),
    }
}

pub fn percent(before: u64, after: u64) -> String {
    if before == 0 {
        return "0.0%".into();
    }
    format!("{:+.1}%", (after as f64 - before as f64) / before as f64 * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use squish_core::Format;

    fn result(path: &str, status: Status, before: u64, after: u64) -> FileResult {
        FileResult {
            input: path.into(),
            output: (status != Status::Failed).then(|| format!("out/{path}").into()),
            status,
            format: Some(Format::Png),
            original_bytes: before,
            output_bytes: after,
            error: (status == Status::Failed).then(|| ("decode_failed", "bad data".to_string())),
            elapsed: Duration::from_millis(84),
        }
    }

    #[test]
    fn renders_like_the_prd_example() {
        let results = [
            result("a.png", Status::Compressed, 182_400_000, 51_900_000),
            result("b.png", Status::AlreadyOptimized, 0, 0),
            result("hero.avif", Status::Unsupported, 10, 0),
            result("x.png", Status::Failed, 0, 0),
            result("y.png", Status::Failed, 0, 0),
        ];
        let text = Summary {
            results: &results,
            problems: &[],
            not_started: 0,
            elapsed: Duration::from_millis(3210),
            jobs: 12,
            dry_run: false,
        }
        .render();
        assert_eq!(
            text,
            "  5 files   182.4 MB -> 51.9 MB   (-71.5%)   3.2 s on 12 threads\n\
             \x20 1 already optimized, kept as is: b.png\n\
             \x20 1 unsupported: hero.avif (PNG)\n\
             \x20 2 failed:\n\
             \x20   x.png: bad data\n\
             \x20   y.png: bad data\n"
        );
    }

    #[test]
    fn json_counts_match_the_text_and_walk_problems_count_as_failed() {
        let results = [
            result("a.png", Status::Compressed, 1000, 400),
            result("b.png", Status::AlreadyOptimized, 50, 50),
            result("x.png", Status::Failed, 0, 0),
        ];
        let problems = [(PathBuf::from("locked"), "permission denied".to_string())];
        let summary = Summary {
            results: &results,
            problems: &problems,
            not_started: 2,
            elapsed: Duration::from_millis(1500),
            jobs: 4,
            dry_run: true,
        };
        let json: serde_json::Value = serde_json::from_str(&summary.json()).unwrap();
        let s = &json["summary"];
        assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!((s["files"].as_u64(), s["compressed"].as_u64(), s["failed"].as_u64()), (Some(4), Some(1), Some(2)));
        assert_eq!((s["original_bytes"].as_u64(), s["output_bytes"].as_u64()), (Some(1050), Some(450)));
        assert_eq!((s["elapsed_ms"].as_u64(), s["not_started"].as_u64()), (Some(1500), Some(2)));
        assert_eq!(s["dry_run"], true);

        let files = json["files"].as_array().unwrap();
        assert_eq!(files[0]["output"], "out/a.png");
        assert_eq!(files[0]["format"], "png");
        assert_eq!(files[0]["elapsed_ms"], 84);
        assert!(files[0].get("error").is_none(), "no error keys on success");
        assert_eq!(files[1]["status"], "already_optimized");
        assert_eq!((&files[2]["error"], &files[2]["output"]), (&"decode_failed".into(), &serde_json::Value::Null));
        assert_eq!((&files[3]["input"], &files[3]["error"]), (&"locked".into(), &"read_failed".into()));
        assert!(summary.render().ends_with("  dry run, nothing written\n"));
    }

    #[test]
    fn units_and_percent() {
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(48_211), "48.2 KB");
        assert_eq!(percent(0, 0), "0.0%");
        assert_eq!(percent(200, 50), "-75.0%");
    }
}

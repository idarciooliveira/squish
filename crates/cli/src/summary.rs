//! The end-of-run summary (PRD FR-11).

use std::fmt::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::run::{FileResult, Status};

pub struct Summary<'a> {
    pub results: &'a [FileResult],
    /// Folders the walk could not read. They count as failed.
    pub problems: &'a [(PathBuf, String)],
    /// Files never started because of Ctrl-C.
    pub not_started: usize,
    pub elapsed: Duration,
    pub jobs: usize,
}

impl Summary<'_> {
    pub fn render(&self) -> String {
        let done = |r: &&FileResult| matches!(r.status, Status::Compressed | Status::AlreadyOptimized);
        let before: u64 = self.results.iter().filter(done).map(|r| r.original_bytes).sum();
        let after: u64 = self.results.iter().filter(done).map(|r| r.output_bytes).sum();
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
        s
    }
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
            status,
            format: Some(Format::Png),
            original_bytes: before,
            output_bytes: after,
            error: (status == Status::Failed).then(|| ("decode_failed", "bad data".to_string())),
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
    fn units_and_percent() {
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(48_211), "48.2 KB");
        assert_eq!(percent(0, 0), "0.0%");
        assert_eq!(percent(200, 50), "-75.0%");
    }
}

//! Runs every PNG, JPEG and WebP in `bench/images/` through `compress`.
//!
//! The corpus is gitignored, so on a fresh clone this prints a message and passes
//! (ADR-0020). Locally it asserts that each output decodes, keeps its dimensions
//! and is never bigger than its input.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use squish_core::{Error, Options, compress, dimensions, inspect};

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/images");
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n != ".gitkeep"))
        .collect();
    files.sort();
    files
}

#[test]
fn bench_corpus_compresses_without_growing() {
    let files = corpus();
    if files.is_empty() {
        eprintln!("bench/images/ is empty, skipping the golden test");
        return;
    }
    let failures = Mutex::new(Vec::<String>::new());
    let queue = Mutex::new(files.iter());
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().min(8));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let Some(path) = queue.lock().unwrap().next() else { return };
                    let name = path.file_name().unwrap().to_string_lossy().to_string();
                    let input = std::fs::read(path).unwrap();
                    let before = inspect(&input).and_then(|i| i.dimensions);
                    match compress(&input, &Options::default()) {
                        Ok(out) => {
                            if out.bytes.len() > input.len() {
                                failures.lock().unwrap().push(format!("{name}: grew"));
                            }
                            let after = dimensions(&out.bytes, out.format);
                            if before != after {
                                failures.lock().unwrap().push(format!("{name}: size {before:?} became {after:?}"));
                            }
                        }
                        // A file over the pixel limit, or one that isn't PNG, JPEG or WebP, is a clean skip.
                        Err(Error::TooManyPixels { .. } | Error::Unsupported(_) | Error::Animated(_)) => {}
                        Err(e) => failures.lock().unwrap().push(format!("{name}: {e}")),
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{} of {} files failed:\n{}", failures.len(), files.len(), failures.join("\n"));
}

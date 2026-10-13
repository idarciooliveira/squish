//! Runs the `squish` binary on the committed core fixtures (ADR-0019 behavior).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures").join(name)
}

/// A temp folder holding copies of the named fixtures, at the given relative paths.
fn workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, name) in files {
        let dest = dir.path().join(rel);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(fixture(name), dest).unwrap();
    }
    dir
}

fn squish(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_squish"))
        .current_dir(dir)
        .args(["--effort", "fast"])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn default_mode_writes_min_files_and_a_second_run_skips_them() {
    let dir =
        workspace(&[("img/a.png", "photo-rgb.png"), ("img/b.jpg", "photo.jpg"), ("img/style.css", "palette.png")]);
    let out = squish(dir.path(), &["img"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let original = fs::metadata(dir.path().join("img/a.png")).unwrap().len();
    let compressed = fs::metadata(dir.path().join("img/a.min.png")).unwrap().len();
    assert!(compressed < original);
    assert!(dir.path().join("img/b.min.jpg").exists());
    assert!(!dir.path().join("img/style.min.css").exists());
    assert!(text(&out.stdout).starts_with("  2 files "), "{}", text(&out.stdout));

    let again = squish(dir.path(), &["img"]);
    assert_eq!(again.status.code(), Some(0));
    assert!(!dir.path().join("img/a.min.min.png").exists());
}

#[test]
fn out_dir_mirrors_the_tree_and_recursion_is_opt_in() {
    let dir = workspace(&[("img/top.png", "photo-rgb.png"), ("img/icons/logo.png", "logo-alpha.png")]);
    let out = squish(dir.path(), &["img", "-o", "out"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(dir.path().join("out/top.png").exists());
    assert!(!dir.path().join("out/icons").exists());

    let out = squish(dir.path(), &["img", "-r", "-o", "out"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(dir.path().join("out/icons/logo.png").exists());
    assert_eq!(fs::read(dir.path().join("img/icons/logo.png")).unwrap(), fs::read(fixture("logo-alpha.png")).unwrap());
}

#[test]
fn in_place_needs_yes_without_a_terminal() {
    let dir = workspace(&[("a.png", "photo-rgb.png")]);
    let out = squish(dir.path(), &["a.png", "--in-place"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(fs::read(dir.path().join("a.png")).unwrap(), fs::read(fixture("photo-rgb.png")).unwrap());

    let out = squish(dir.path(), &["a.png", "--in-place", "--yes"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        fs::metadata(dir.path().join("a.png")).unwrap().len() < fs::metadata(fixture("photo-rgb.png")).unwrap().len()
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no .min file and no temp file");
}

#[test]
fn output_collisions_stop_before_any_work() {
    let dir = workspace(&[("a/logo.png", "logo-alpha.png"), ("b/logo.png", "logo-alpha.png")]);
    let out = squish(dir.path(), &["a/logo.png", "b/logo.png", "-o", "out"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("would both be written to"));
    assert!(!dir.path().join("out").exists());
}

#[test]
fn unsupported_files_exit_0_and_broken_ones_exit_1() {
    let dir = workspace(&[("anim.png", "animated.png")]);
    fs::write(dir.path().join("notes.txt"), "hello").unwrap();
    let out = squish(dir.path(), &["anim.png", "notes.txt"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("2 unsupported:"), "{stdout}");
    assert!(!dir.path().join("anim.min.png").exists());

    let mut broken = fs::read(fixture("photo-rgb.png")).unwrap();
    broken.truncate(200);
    fs::write(dir.path().join("broken.png"), broken).unwrap();
    let out = squish(dir.path(), &["broken.png", "missing.png"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("2 failed:"), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("broken.png"));
}

#[test]
fn already_optimized_files_still_get_a_copy() {
    let dir = workspace(&[("a.png", "photo-rgb.png")]);
    assert_eq!(squish(dir.path(), &["a.png", "-o", "once"]).status.code(), Some(0));
    let out = squish(dir.path(), &["once/a.png", "-o", "twice", "--lossless"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(dir.path().join("twice/a.png").exists());
}

#[test]
fn stdin_to_stdout() {
    let input = fs::read(fixture("photo.jpg")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_squish"))
        .args(["--effort", "fast", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(out.stdout.starts_with(&[0xFF, 0xD8]));
    assert!(out.stdout.len() < input.len());

    let dir = tempfile::tempdir().unwrap();
    assert_eq!(squish(dir.path(), &["-", "a.png"]).status.code(), Some(2));
}

#[test]
fn serve_is_reserved() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_squish")).arg("serve").current_dir(dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn bad_flags_are_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(squish(dir.path(), &["--quality", "101", "a.png"]).status.code(), Some(2));
    assert_eq!(squish(dir.path(), &["--metadata", "some", "a.png"]).status.code(), Some(2));
    assert_eq!(squish(dir.path(), &[]).status.code(), Some(2));
}

/// Runs `squish -` with `input` on stdin.
fn squish_stdin(input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_squish"))
        .args(["--effort", "fast", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn stdin_passes_unsupported_input_through_and_fails_on_broken_input() {
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;";
    let out = squish_stdin(gif);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(out.stdout, gif);

    let mut broken = fs::read(fixture("photo-rgb.png")).unwrap();
    broken.truncate(200);
    let out = squish_stdin(&broken);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "nothing goes to stdout on failure");
}

#[test]
fn quiet_prints_only_errors() {
    let dir = workspace(&[("a.png", "photo-rgb.png")]);
    let out = squish(dir.path(), &["-q", "a.png"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(dir.path().join("a.min.png").exists());

    let out = squish(dir.path(), &["-q", "missing.png"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(text(&out.stderr).contains("missing.png"));
}

#[test]
fn max_pixels_fails_big_images_without_writing() {
    // photo-rgb.png is 192x128, about 0.025 MP.
    let dir = workspace(&[("a.png", "photo-rgb.png")]);
    let out = squish(dir.path(), &["--max-pixels", "0.01", "a.png"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("above the limit"), "{}", text(&out.stdout));
    assert!(!dir.path().join("a.min.png").exists());
    assert_eq!(squish(dir.path(), &["--max-pixels", "0.03", "a.png"]).status.code(), Some(0));
}

#[test]
fn output_does_not_depend_on_the_job_count() {
    let files =
        [("a.png", "photo-rgb.png"), ("b.png", "logo-alpha.png"), ("c.jpg", "photo.jpg"), ("d.jpg", "photo-gray.jpg")];
    let dir = workspace(&files);
    assert_eq!(squish(dir.path(), &[".", "-j", "1", "-o", "one"]).status.code(), Some(0));
    assert_eq!(squish(dir.path(), &[".", "-j", "4", "-o", "four"]).status.code(), Some(0));
    for (name, _) in files {
        assert_eq!(
            fs::read(dir.path().join("one").join(name)).unwrap(),
            fs::read(dir.path().join("four").join(name)).unwrap(),
            "{name}"
        );
    }
}

#[cfg(unix)]
#[test]
fn write_failures_exit_1_and_name_the_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = workspace(&[("a.png", "photo-rgb.png")]);
    let out_dir = dir.path().join("out");
    fs::create_dir(&out_dir).unwrap();
    fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o555)).unwrap();
    if fs::write(out_dir.join("probe"), b"x").is_ok() {
        eprintln!("running as root, a read-only folder can't make the write fail");
        return;
    }
    let out = squish(dir.path(), &["a.png", "-o", "out"]);
    fs::set_permissions(&out_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stdout).contains("1 failed"), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("could not write"), "{}", text(&out.stderr));
    assert_eq!(fs::read_dir(&out_dir).unwrap().count(), 0, "no temp file left behind");
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_new_files_and_exits_130() {
    let copies = 300;
    let dir = tempfile::tempdir().unwrap();
    for i in 0..copies {
        fs::copy(fixture("photo.jpg"), dir.path().join(format!("{i:03}.jpg"))).unwrap();
    }
    let child = Command::new(env!("CARGO_BIN_EXE_squish"))
        .current_dir(dir.path())
        .args([".", "-j", "1", "-o", "out"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The handler is installed before the first file starts, so a written file means it's live.
    let started = std::time::Instant::now();
    while fs::read_dir(dir.path().join("out")).map_or(0, |d| d.count()) == 0 {
        assert!(started.elapsed().as_secs() < 60, "no output after 60 s");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let kill = Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    assert!(kill.success());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(130), "{}", text(&out.stderr));
    let written: Vec<String> = fs::read_dir(dir.path().join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(written.len() < copies, "every file was processed, the interrupt came too late");
    assert!(written.iter().all(|n| n.ends_with(".jpg") && !n.starts_with('.')), "partial files: {written:?}");
}

/// Entry names and bytes of a zip, in archive order.
fn zip_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut entry = zip.by_index(i).unwrap();
            let name = entry.name().unwrap().into_owned();
            assert_eq!(entry.compression(), zip::CompressionMethod::Stored, "{name}");
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect()
}

#[test]
fn dry_run_writes_nothing_in_any_mode() {
    let dir = workspace(&[("a.png", "photo-rgb.png"), ("b.jpg", "photo.jpg")]);
    let out = squish(dir.path(), &[".", "-n"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("dry run, nothing written"), "{}", text(&out.stdout));
    assert!(text(&out.stdout).contains("-> "), "the savings are still reported");

    // --in-place doesn't ask, because nothing gets overwritten.
    for args in [&["-o", "out"][..], &["--zip", "all.zip"], &["--in-place"]] {
        let out = squish(dir.path(), &[&[".", "--dry-run"], args].concat());
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", text(&out.stderr));
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2, "only the two inputs");
    assert_eq!(fs::read(dir.path().join("a.png")).unwrap(), fs::read(fixture("photo-rgb.png")).unwrap());
}

#[test]
fn json_report_follows_adr_0019() {
    let dir = workspace(&[("img/a.png", "photo-rgb.png"), ("img/anim.png", "animated.png")]);
    let out = squish(dir.path(), &["img", "missing.png", "-o", "out", "--json"]);
    assert_eq!(out.status.code(), Some(1), "missing.png fails the run");
    let report: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    assert_eq!(report["version"], env!("CARGO_PKG_VERSION"));
    let summary = &report["summary"];
    for (key, want) in [("files", 3), ("compressed", 1), ("unsupported", 1), ("failed", 1), ("already_optimized", 0)] {
        assert_eq!(summary[key], want, "{key}");
    }
    assert_eq!(summary["dry_run"], false);
    assert!(summary["output_bytes"].as_u64() < summary["original_bytes"].as_u64());

    let files = report["files"].as_array().unwrap();
    let file = |input: &str| files.iter().find(|f| f["input"] == input).unwrap_or_else(|| panic!("{input}"));
    let a = file("img/a.png");
    assert_eq!((&a["status"], &a["format"], &a["output"]), (&"compressed".into(), &"png".into(), &"out/a.png".into()));
    assert_eq!(a["output_bytes"].as_u64(), Some(fs::metadata(dir.path().join("out/a.png")).unwrap().len()));
    assert!(a.get("error").is_none());
    let anim = file("img/anim.png");
    assert_eq!((&anim["status"], &anim["error"]), (&"unsupported".into(), &"animated_image".into()));
    assert!(anim["output"].is_null());
    assert_eq!(file("missing.png")["error"], "read_failed");

    // With --dry-run, output still names the path that would have been written.
    let out = squish(dir.path(), &["img/a.png", "-o", "dry", "-n", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    assert_eq!(report["files"][0]["output"], "dry/a.png");
    assert!(!dir.path().join("dry").exists());
}

#[test]
fn zip_holds_the_same_files_as_out_dir() {
    let dir = workspace(&[
        ("img/top.png", "photo-rgb.png"),
        ("img/icons/logo.png", "logo-alpha.png"),
        ("img/photo.jpg", "photo.jpg"),
        ("img/anim.png", "animated.png"),
    ]);
    let out = squish(dir.path(), &["img", "-r", "--zip", "dist/all.zip"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(squish(dir.path(), &["img", "-r", "-o", "out"]).status.code(), Some(0));

    let mut entries = zip_entries(&dir.path().join("dist/all.zip"));
    entries.sort();
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["icons/logo.png", "photo.jpg", "top.png"], "unsupported files stay out");
    for (name, bytes) in &entries {
        assert_eq!(bytes, &fs::read(dir.path().join("out").join(name)).unwrap(), "{name}");
    }
    assert_eq!(fs::read_dir(dir.path().join("dist")).unwrap().count(), 1, "no temp file left behind");

    // Already-optimized files still go in, so the zip is complete.
    let out = squish(dir.path(), &["out/top.png", "--lossless", "--zip", "again.zip", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    assert_eq!(report["files"][0]["status"], "already_optimized");
    assert_eq!(report["files"][0]["output"], "top.png");
    assert_eq!(zip_entries(&dir.path().join("again.zip")).len(), 1);
}

#[test]
fn zip_collisions_and_conflicts_are_usage_errors() {
    let dir = workspace(&[("a/logo.png", "logo-alpha.png"), ("b/logo.png", "logo-alpha.png")]);
    let out = squish(dir.path(), &["a/logo.png", "b/logo.png", "--zip", "all.zip"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("zip entry logo.png"), "{}", text(&out.stderr));
    assert_eq!(squish(dir.path(), &["a", "--zip", "all.zip", "-o", "out"]).status.code(), Some(2));
    assert_eq!(squish(dir.path(), &["a/logo.png", "--zip", "a/logo.png"]).status.code(), Some(2));
    assert_eq!(squish(dir.path(), &["-", "--zip", "all.zip"]).status.code(), Some(2));
    assert_eq!(squish(dir.path(), &["-", "--json"]).status.code(), Some(2));
    assert!(!dir.path().join("all.zip").exists() && !dir.path().join("out").exists());
}

#[test]
fn stdin_dry_run_reports_without_writing_stdout() {
    let input = fs::read(fixture("photo.jpg")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_squish"))
        .args(["--effort", "fast", "-n", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(out.stdout.is_empty());
    assert!(text(&out.stderr).contains("-> "), "{}", text(&out.stderr));
}

#[cfg(unix)]
#[test]
fn ctrl_c_during_zip_keeps_a_valid_zip_of_the_finished_files() {
    let copies = 300;
    let dir = tempfile::tempdir().unwrap();
    for i in 0..copies {
        fs::copy(fixture("photo.jpg"), dir.path().join(format!("{i:03}.jpg"))).unwrap();
    }
    let child = Command::new(env!("CARGO_BIN_EXE_squish"))
        .current_dir(dir.path())
        .args([".", "-j", "1", "--zip", "out/all.zip"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The temp zip grows as entries land, so a non-empty one means the handler is live.
    let started = std::time::Instant::now();
    let temp_size = || {
        fs::read_dir(dir.path().join("out"))
            .map(|d| d.map(|e| e.unwrap().metadata().unwrap().len()).sum::<u64>())
            .unwrap_or(0)
    };
    while temp_size() < 1000 {
        assert!(started.elapsed().as_secs() < 60, "no zip entry after 60 s");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let kill = Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    assert!(kill.success());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(130), "{}", text(&out.stderr));

    let entries = zip_entries(&dir.path().join("out/all.zip"));
    assert!(!entries.is_empty() && entries.len() < copies, "{} entries", entries.len());
    assert!(entries.iter().all(|(_, bytes)| bytes.starts_with(&[0xFF, 0xD8])));
    assert_eq!(fs::read_dir(dir.path().join("out")).unwrap().count(), 1, "no temp file left behind");
}

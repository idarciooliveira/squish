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

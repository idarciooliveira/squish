//! Turns command-line paths into the list of files to compress (ADR-0019).
//!
//! A file named on the command line is always tried. Folder walks only pick image
//! extensions, skip `*.min.*`, hidden entries and symlinks, and go one level deep
//! unless `-r` is set.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Extensions a folder walk picks up. The format itself is decided by magic bytes.
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "webp", "avif", "gif", "apng"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub path: PathBuf,
    /// Where the file lands under `-o`: relative to the folder argument it came from,
    /// or just the file name for a file passed directly.
    pub rel: PathBuf,
}

/// What `collect` found: files to compress, and folders it could not read.
#[derive(Debug, Default)]
pub struct Collected {
    pub inputs: Vec<Input>,
    pub problems: Vec<(PathBuf, String)>,
}

/// Expands `args` into files. `exclude` is a folder the walk never enters, the `-o`
/// target, so a second run doesn't pick up the first run's outputs.
pub fn collect(args: &[PathBuf], recursive: bool, exclude: Option<&Path>) -> Result<Collected, String> {
    let exclude = exclude.and_then(|p| fs::canonicalize(p).ok());
    let mut found = Collected::default();
    for arg in args {
        if arg.exists() || !has_glob_chars(arg) {
            add_arg(arg, recursive, exclude.as_deref(), &mut found);
            continue;
        }
        // The shell left the pattern alone, which is what happens on Windows.
        let pattern = arg.to_string_lossy();
        let matches = glob::glob(&pattern).map_err(|e| format!("invalid pattern {pattern:?}: {e}"))?;
        let mut any = false;
        for entry in matches {
            match entry {
                Ok(path) => {
                    any = true;
                    add_arg(&path, recursive, exclude.as_deref(), &mut found);
                }
                Err(e) => found.problems.push((e.path().to_path_buf(), e.error().to_string())),
            }
        }
        if !any {
            return Err(format!("no files match {pattern:?}"));
        }
    }

    let mut seen = HashSet::new();
    found.inputs.retain(|input| seen.insert(fs::canonicalize(&input.path).unwrap_or_else(|_| input.path.clone())));
    Ok(found)
}

fn add_arg(arg: &Path, recursive: bool, exclude: Option<&Path>, found: &mut Collected) {
    if arg.is_dir() {
        if let Err(e) = walk(arg, arg, recursive, exclude, found) {
            found.problems.push((arg.to_path_buf(), e.to_string()));
        }
    } else {
        // Missing files are kept too, so they show up as failed instead of vanishing.
        let rel = arg.file_name().map_or_else(|| arg.to_path_buf(), PathBuf::from);
        found.inputs.push(Input { path: arg.to_path_buf(), rel });
    }
}

fn walk(
    root: &Path,
    dir: &Path,
    recursive: bool,
    exclude: Option<&Path>,
    found: &mut Collected,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        // `DirEntry::file_type` doesn't follow symlinks, so links are neither files nor folders here.
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !recursive || exclude.is_some_and(|ex| fs::canonicalize(&path).is_ok_and(|p| p == ex)) {
                continue;
            }
            if let Err(e) = walk(root, &path, recursive, exclude, found) {
                found.problems.push((path, e.to_string()));
            }
        } else if kind.is_file() && is_image_name(&path) && !is_min_name(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            found.inputs.push(Input { path, rel });
        }
    }
    Ok(())
}

fn has_glob_chars(path: &Path) -> bool {
    path.to_string_lossy().contains(['*', '?', '['])
}

fn is_image_name(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// `photo.min.png`, the name squish writes by default.
fn is_min_name(path: &Path) -> bool {
    path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.to_ascii_lowercase().ends_with(".min"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for f in files {
            let path = dir.path().join(f);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        dir
    }

    fn rels(found: &Collected) -> Vec<String> {
        found.inputs.iter().map(|i| i.rel.to_string_lossy().replace('\\', "/")).collect()
    }

    #[test]
    fn walk_picks_images_and_skips_the_rest() {
        let dir = tree(&["a.png", "b.JPG", "c.css", "d.min.png", ".hidden.png", ".git/e.png", "sub/f.jpeg"]);
        let found = collect(&[dir.path().to_path_buf()], false, None).unwrap();
        assert_eq!(rels(&found), ["a.png", "b.JPG"]);
        let found = collect(&[dir.path().to_path_buf()], true, None).unwrap();
        assert_eq!(rels(&found), ["a.png", "b.JPG", "sub/f.jpeg"]);
    }

    #[test]
    fn named_files_are_always_tried() {
        let dir = tree(&["notes.txt", "x.min.png"]);
        let args = [dir.path().join("notes.txt"), dir.path().join("x.min.png"), dir.path().join("missing.png")];
        let found = collect(&args, false, None).unwrap();
        assert_eq!(rels(&found), ["notes.txt", "x.min.png", "missing.png"]);
    }

    #[test]
    fn the_same_file_is_processed_once() {
        let dir = tree(&["a.png"]);
        let args = [dir.path().to_path_buf(), dir.path().join("a.png"), dir.path().join("./a.png")];
        assert_eq!(collect(&args, false, None).unwrap().inputs.len(), 1);
    }

    #[test]
    fn walk_skips_the_output_folder() {
        let dir = tree(&["a.png", "out/a.png"]);
        let found = collect(&[dir.path().to_path_buf()], true, Some(&dir.path().join("out"))).unwrap();
        assert_eq!(rels(&found), ["a.png"]);
    }

    #[test]
    fn unexpanded_globs_are_expanded() {
        let dir = tree(&["a.png", "b.png", "c.jpg"]);
        let found = collect(&[dir.path().join("*.png")], false, None).unwrap();
        assert_eq!(rels(&found), ["a.png", "b.png"]);
        assert!(collect(&[dir.path().join("*.gif")], false, None).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn walk_does_not_follow_symlinks() {
        let dir = tree(&["real/a.png"]);
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("real/a.png"), dir.path().join("b.png")).unwrap();
        let found = collect(&[dir.path().to_path_buf()], true, None).unwrap();
        assert_eq!(rels(&found), ["real/a.png"]);
    }
}

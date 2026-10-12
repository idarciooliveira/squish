//! Where each output goes, and the collision check that runs before any work (ADR-0019).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::inputs::Input;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// `name.min.ext` next to the input.
    Suffix,
    /// Mirror the inputs into this folder with their original names.
    OutDir(PathBuf),
    /// Replace the inputs.
    InPlace,
}

impl Mode {
    pub fn output_path(&self, input: &Input) -> PathBuf {
        match self {
            Mode::Suffix => min_name(&input.path),
            Mode::OutDir(dir) => dir.join(&input.rel),
            Mode::InPlace => input.path.clone(),
        }
    }
}

/// `images/logo.png` becomes `images/logo.min.png`. The extension stays as written.
fn min_name(path: &Path) -> PathBuf {
    let mut name = OsString::from(path.file_stem().unwrap_or(path.as_os_str()));
    name.push(".min");
    if let Some(ext) = path.extension() {
        name.push(".");
        name.push(ext);
    }
    path.with_file_name(name)
}

/// Output paths for `inputs`, in order. Fails if two inputs land on the same path, or if
/// an output would overwrite an input outside `--in-place`.
pub fn outputs(inputs: &[Input], mode: &Mode) -> Result<Vec<PathBuf>, String> {
    let outputs: Vec<PathBuf> = inputs.iter().map(|i| mode.output_path(i)).collect();
    if *mode == Mode::InPlace {
        return Ok(outputs);
    }
    let sources: HashMap<PathBuf, &Path> = inputs.iter().map(|i| (key(&i.path), i.path.as_path())).collect();
    let mut taken: HashMap<PathBuf, &Path> = HashMap::new();
    for (input, output) in inputs.iter().zip(&outputs) {
        let k = key(output);
        if let Some(source) = sources.get(&k) {
            return Err(format!(
                "{} would overwrite the input {}; use --in-place to replace originals",
                input.path.display(),
                source.display()
            ));
        }
        if let Some(first) = taken.insert(k, &input.path) {
            return Err(format!(
                "{} and {} would both be written to {}",
                first.display(),
                input.path.display(),
                output.display()
            ));
        }
    }
    Ok(outputs)
}

/// A comparable form of a path that may not exist yet: the real parent folder plus the name.
fn key(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    match (absolute.parent().and_then(|p| fs::canonicalize(p).ok()), absolute.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => absolute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(path: &str, rel: &str) -> Input {
        Input { path: PathBuf::from(path), rel: PathBuf::from(rel) }
    }

    #[test]
    fn modes_map_paths_like_adr_0019() {
        let logo = input("images/icons/logo.png", "icons/logo.png");
        assert_eq!(Mode::Suffix.output_path(&logo), Path::new("images/icons/logo.min.png"));
        assert_eq!(Mode::OutDir("out".into()).output_path(&logo), Path::new("out/icons/logo.png"));
        assert_eq!(Mode::InPlace.output_path(&logo), Path::new("images/icons/logo.png"));
    }

    #[test]
    fn suffix_keeps_the_extension_as_written() {
        assert_eq!(min_name(Path::new("a/photo.JPEG")), Path::new("a/photo.min.JPEG"));
        assert_eq!(min_name(Path::new("photo.min.png")), Path::new("photo.min.min.png"));
        assert_eq!(min_name(Path::new("noext")), Path::new("noext.min"));
    }

    #[test]
    fn two_inputs_on_one_output_is_an_error() {
        let inputs = [input("a/logo.png", "logo.png"), input("b/logo.png", "logo.png")];
        let err = outputs(&inputs, &Mode::OutDir("out".into())).unwrap_err();
        assert!(err.contains("would both be written to"), "{err}");
        assert!(outputs(&inputs, &Mode::Suffix).is_ok());
    }

    #[test]
    fn an_output_on_top_of_an_input_is_an_error() {
        let inputs = [input("x/a.png", "a.png"), input("x/a.min.png", "a.min.png")];
        assert!(outputs(&inputs, &Mode::Suffix).unwrap_err().contains("would overwrite the input"));
        let inputs = [input("x/a.png", "a.png")];
        assert!(outputs(&inputs, &Mode::OutDir("x".into())).is_err());
        assert!(outputs(&inputs, &Mode::InPlace).is_ok());
    }
}

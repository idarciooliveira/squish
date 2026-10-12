//! Atomic file writes: a temp file in the target folder, then a rename (ADR-0019).
//!
//! An interrupted run never leaves half a file behind.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `dest`, creating its folder. With `keep_permissions`, the new file
/// gets that file's permissions, which `--in-place` uses to keep the original's mode.
pub fn atomic_write(dest: &Path, bytes: &[u8], keep_permissions: Option<&Path>) -> io::Result<()> {
    let dir = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)?;
    let tmp = temp_path(dir, dest);
    let result = (|| {
        let mut file = File::create_new(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Some(source) = keep_permissions {
            fs::set_permissions(&tmp, fs::metadata(source)?.permissions())?;
        }
        fs::rename(&tmp, dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn temp_path(dir: &Path, dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{name}.squish-{}-{n}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_replaces_without_leaving_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested/out.png");
        atomic_write(&dest, b"one", None).unwrap();
        atomic_write(&dest, b"two", None).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"two");
        assert_eq!(fs::read_dir(dest.parent().unwrap()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_source_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.png");
        fs::write(&src, b"x").unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&src, b"y", Some(&src)).unwrap();
        assert_eq!(fs::metadata(&src).unwrap().permissions().mode() & 0o777, 0o640);
    }
}

//! The `--zip` output: one STORE zip, built in a temp file and renamed at the end (ADR-0008, ADR-0019).
//!
//! Workers add entries as they finish, so the zip never holds more than one file in memory.
//! Entries land in completion order. Each entry's bytes are the same for any job count.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::write::temp_path;

pub struct Archive {
    dest: PathBuf,
    tmp: PathBuf,
    state: Mutex<State>,
}

struct State {
    writer: Option<ZipWriter<File>>,
    /// The first write error. Once set, the zip is broken and every later entry fails.
    error: Option<String>,
}

impl Archive {
    /// Creates the temp file next to `dest`, creating `dest`'s folder.
    pub fn create(dest: &Path) -> io::Result<Archive> {
        let dir = match dest.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        fs::create_dir_all(dir)?;
        let tmp = temp_path(dir, dest);
        let file = File::create_new(&tmp)?;
        Ok(Archive {
            dest: dest.to_path_buf(),
            tmp,
            state: Mutex::new(State { writer: Some(ZipWriter::new(file)), error: None }),
        })
    }

    pub fn add(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        let writer = state.writer.as_mut().expect("the zip is open until finish");
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(bytes.len() as u64 >= u64::from(u32::MAX));
        let result = writer.start_file(name, options).map_err(io::Error::from).and_then(|()| writer.write_all(bytes));
        result.map_err(|e| {
            let message = e.to_string();
            state.error = Some(message.clone());
            message
        })
    }

    /// Writes the central directory and renames the zip into place. On any error,
    /// including one from an earlier `add`, the temp file is removed and `dest` is untouched.
    pub fn finish(self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(error) = state.error.take() {
            return Err(io::Error::other(error));
        }
        let writer = state.writer.take().expect("finish runs once");
        let file = writer.finish().map_err(io::Error::from)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.tmp, &self.dest)
    }
}

impl Drop for Archive {
    /// Nothing left to do after a successful rename. Otherwise this removes the temp file.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn stores_entries_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested/all.zip");
        let archive = Archive::create(&dest).unwrap();
        archive.add("a.png", b"one").unwrap();
        archive.add("icons/b.png", b"two").unwrap();
        archive.finish().unwrap();
        assert_eq!(fs::read_dir(dest.parent().unwrap()).unwrap().count(), 1);

        let mut zip = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let mut entry = zip.by_name("icons/b.png").unwrap();
        assert_eq!(entry.compression(), CompressionMethod::Stored);
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"two");
    }

    #[test]
    fn a_failed_add_fails_the_zip_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("all.zip");
        let archive = Archive::create(&dest).unwrap();
        archive.add("a.png", b"one").unwrap();
        // The zip crate refuses a second entry with the same name.
        assert!(archive.add("a.png", b"two").is_err());
        assert!(archive.add("b.png", b"three").is_err(), "a broken zip takes no more entries");
        assert!(archive.finish().is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn dropping_an_unfinished_zip_removes_the_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        drop(Archive::create(&dir.path().join("all.zip")).unwrap());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

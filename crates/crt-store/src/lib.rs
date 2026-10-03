//! `ReadingStore` that keeps one JSON file per reading under a directory
//! the composition root chooses (the user's cache directory). Files are
//! written to a temporary name and renamed, so a crash never leaves half a
//! reading behind.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crt_app::{ReadingKey, ReadingStore, StoreError};
use crt_domain::Reading;
use crt_wire::ReadingDto;

/// Version of the on-disk shape. Files with another version are ignored
/// (and overwritten on the next read), never misread.
/// 1: lines absolute in the file (never released). 2: lines relative to the
/// function's first line. 3: scenarios may be absent (`null`).
const FORMAT: u32 = 3;

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    format: u32,
    reading: ReadingDto,
}

pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, key: &ReadingKey) -> PathBuf {
        self.root.join(safe(key.language.as_str())).join(format!(
            "{}.{}.{}.json",
            key.function_hash,
            safe(&key.author.model),
            safe(&key.author.prompt)
        ))
    }
}

impl ReadingStore for FileStore {
    fn get(&self, key: &ReadingKey) -> Result<Option<Reading>, StoreError> {
        let path = self.path_for(key);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(err(&path, e)),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| err(&path, e))?;
        if value.get("format").and_then(serde_json::Value::as_u64) != Some(u64::from(FORMAT)) {
            return Ok(None);
        }
        let stored: Stored = serde_json::from_value(value).map_err(|e| err(&path, e))?;
        let reading = Reading::try_from(stored.reading).map_err(|e| err(&path, e))?;
        // A file renamed or copied by hand must not answer for other source.
        if reading.function_hash != key.function_hash || reading.author != key.author {
            return Ok(None);
        }
        Ok(Some(reading))
    }

    fn put(&self, key: &ReadingKey, reading: &Reading) -> Result<(), StoreError> {
        let path = self.path_for(key);
        let dir = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(dir).map_err(|e| err(dir, e))?;
        let stored = Stored {
            format: FORMAT,
            reading: ReadingDto::from(reading),
        };
        let json = serde_json::to_vec_pretty(&stored).map_err(|e| err(&path, e))?;
        // A uniquely named temporary file in the same directory, renamed
        // into place: concurrent writers never share it, and a failure
        // removes it.
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| err(dir, e))?;
        tmp.write_all(&json).map_err(|e| err(tmp.path(), e))?;
        tmp.as_file().sync_all().map_err(|e| err(tmp.path(), e))?;
        tmp.persist(&path)
            .map(|_| ())
            .map_err(|e| err(&path, e.error))
    }
}

/// Keeps a path component to ASCII letters, digits, `-`, `_` and `.`.
fn safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_start_matches('.')
        .to_string()
}

fn err(path: &Path, e: impl std::fmt::Display) -> StoreError {
    StoreError(format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crt_domain::{Author, CheckReport, ContentHash, LanguageId};

    fn key(model: &str) -> ReadingKey {
        ReadingKey {
            language: LanguageId::new("go"),
            function_hash: ContentHash::of(b"f"),
            author: Author {
                model: model.into(),
                prompt: "v1".into(),
            },
        }
    }

    fn reading(model: &str) -> Reading {
        Reading {
            function_hash: ContentHash::of(b"f"),
            author: Author {
                model: model.into(),
                prompt: "v1".into(),
            },
            notes: vec![],
            scenarios: None,
            check: CheckReport::default(),
        }
    }

    #[test]
    fn put_then_get_and_miss_for_another_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        assert_eq!(store.get(&key("a/b:c")).unwrap(), None);
        store.put(&key("a/b:c"), &reading("a/b:c")).unwrap();
        assert_eq!(store.get(&key("a/b:c")).unwrap(), Some(reading("a/b:c")));
        assert_eq!(store.get(&key("other")).unwrap(), None);
    }

    #[test]
    fn model_names_cannot_escape_the_store_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let path = store.path_for(&key("../../etc/passwd"));
        assert!(path.starts_with(dir.path()));
        assert_eq!(path.parent().unwrap(), dir.path().join("go"));
    }

    #[test]
    fn a_file_in_another_format_is_a_miss_not_a_misread() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let path = store.path_for(&key("m"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The unversioned shape written before formats existed.
        let old = serde_json::to_vec(&ReadingDto::from(&reading("m"))).unwrap();
        fs::write(&path, old).unwrap();
        assert_eq!(store.get(&key("m")).unwrap(), None);
        store.put(&key("m"), &reading("m")).unwrap();
        assert_eq!(store.get(&key("m")).unwrap(), Some(reading("m")));
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_a_reading() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let path = store.path_for(&key("m"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{not json").unwrap();
        assert!(store.get(&key("m")).is_err());
    }

    #[test]
    fn a_file_that_answers_for_other_source_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let mut wrong = reading("m");
        wrong.function_hash = ContentHash::of(b"other");
        store.put(&key("m"), &wrong).unwrap();
        assert_eq!(store.get(&key("m")).unwrap(), None);
    }
}

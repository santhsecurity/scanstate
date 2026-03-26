//! Write-ahead logging mechanism and structures.

use crate::checkpoint::ScanStateError;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Append-only journal entry for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Scanner-defined stable target identifier.
    pub target_id: String,
    /// Generic status label such as `completed`, `skipped`, or `failed`.
    pub status: String,
    /// UNIX timestamp in seconds.
    pub timestamp: u64,
    /// Findings emitted while processing this target.
    pub findings_count: usize,
}

/// Newline-delimited write-ahead journal.
#[derive(Debug, Clone)]
pub struct WriteAheadJournal {
    path: PathBuf,
}

impl WriteAheadJournal {
    /// Create a journal bound to a file path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Append a journal entry and fsync it for crash recovery.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the entry cannot be written to disk,
    /// or `ScanStateError::Serde` if serialization fails.
    pub fn append(&self, entry: &Entry) -> Result<(), ScanStateError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| ScanStateError::Io { path: parent.to_path_buf(), source: e })?;
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
            
        let mut buf = serde_json::to_vec(entry)?;
        buf.push(b'\n');
        file.write_all(&buf).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
        file.sync_all().map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
        Ok(())
    }

    /// Read all journal entries in append order.
    ///
    /// Fails on the first corrupt entry. Use [`Self::replay_lenient`] to skip
    /// corrupt entries and recover as much as possible.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the file cannot be read,
    /// or `ScanStateError::Serde` if a journal entry is corrupt.
    pub fn replay(&self) -> Result<Vec<Entry>, ScanStateError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = OpenOptions::new().read(true).open(&self.path).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();
        let mut buf = Vec::new();

        while reader.read_until(b'\n', &mut buf).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })? > 0 {
            if buf.iter().all(|b| b.is_ascii_whitespace()) {
                buf.clear();
                continue;
            }
            entries.push(serde_json::from_slice(&buf)?);
            buf.clear();
        }

        Ok(entries)
    }

    /// Read all valid journal entries, skipping corrupt ones.
    ///
    /// Returns `(valid_entries, corrupt_line_count)`. Use this for crash
    /// recovery where partial data is better than no data.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the file cannot be read.
    /// Parsing errors are counted as corrupt lines rather than returning an error.
    pub fn replay_lenient(&self) -> Result<(Vec<Entry>, usize), ScanStateError> {
        if !self.path.exists() {
            return Ok((Vec::new(), 0));
        }

        let file = OpenOptions::new().read(true).open(&self.path).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();
        let mut corrupt_count = 0;
        let mut buf = Vec::new();

        while reader.read_until(b'\n', &mut buf).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })? > 0 {
            if buf.iter().all(|b| b.is_ascii_whitespace()) {
                buf.clear();
                continue;
            }
            match serde_json::from_slice::<Entry>(&buf) {
                Ok(entry) => entries.push(entry),
                Err(_) => corrupt_count += 1,
            }
            buf.clear();
        }

        Ok((entries, corrupt_count))
    }

    /// Clear the journal after successful completion.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the journal file cannot be truncated.
    pub fn truncate(&self) -> Result<(), ScanStateError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| ScanStateError::Io { path: parent.to_path_buf(), source: e })?;
            }
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path).map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
        file.sync_all().map_err(|e| ScanStateError::Io { path: self.path.clone(), source: e })?;
        Ok(())
    }

    /// File path used by this journal.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, WriteAheadJournal};
    use crate::checkpoint::ScanStateError;

    fn sample_entry(target_id: &str, status: &str, timestamp: u64, findings_count: usize) -> Entry {
        Entry {
            target_id: target_id.to_string(),
            status: status.to_string(),
            timestamp,
            findings_count,
        }
    }

    #[test]
    fn append_writes_entry_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let journal = WriteAheadJournal::new(dir.path().join("scan.log"));
        let entry = sample_entry("target-a", "completed", 100, 2);

        journal.append(&entry).unwrap();

        let content = std::fs::read_to_string(journal.path()).unwrap();
        assert!(content.contains("\"target_id\":\"target-a\""));
        assert!(content.contains("\"status\":\"completed\""));
    }

    #[test]
    fn replay_returns_empty_for_missing_journal() {
        let dir = tempfile::tempdir().unwrap();
        let journal = WriteAheadJournal::new(dir.path().join("missing.log"));

        let entries = journal.replay().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn replay_returns_entries_in_append_order() {
        let dir = tempfile::tempdir().unwrap();
        let journal = WriteAheadJournal::new(dir.path().join("scan.log"));
        let first = sample_entry("target-a", "completed", 100, 1);
        let second = sample_entry("target-b", "skipped", 101, 0);

        journal.append(&first).unwrap();
        journal.append(&second).unwrap();

        let entries = journal.replay().unwrap();
        assert_eq!(entries, vec![first, second]);
    }

    #[test]
    fn replay_returns_error_for_corrupt_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scan.log");
        std::fs::write(&path, "{\"target_id\":").unwrap();
        let journal = WriteAheadJournal::new(path);

        let err = journal.replay().unwrap_err();
        assert!(matches!(err, ScanStateError::Serde(_)));
    }

    #[test]
    fn replay_lenient_skips_corrupt_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scan.log");
        let valid = serde_json::to_string(&sample_entry("target-a", "completed", 100, 1)).unwrap();
        let content = format!("{valid}\n{{corrupt garbage}}\n{valid}\n");
        std::fs::write(&path, content).unwrap();

        let journal = WriteAheadJournal::new(path);
        let (entries, corrupt) = journal.replay_lenient().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(corrupt, 1);
    }

    #[test]
    fn replay_lenient_empty_journal() {
        let dir = tempfile::tempdir().unwrap();
        let journal = WriteAheadJournal::new(dir.path().join("missing.log"));
        let (entries, corrupt) = journal.replay_lenient().unwrap();
        assert!(entries.is_empty());
        assert_eq!(corrupt, 0);
    }

    #[test]
    fn replay_lenient_all_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scan.log");
        std::fs::write(&path, "not json\nalso not json\n").unwrap();

        let journal = WriteAheadJournal::new(path);
        let (entries, corrupt) = journal.replay_lenient().unwrap();
        assert!(entries.is_empty());
        assert_eq!(corrupt, 2);
    }

    #[test]
    fn truncate_clears_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let journal = WriteAheadJournal::new(dir.path().join("scan.log"));
        journal
            .append(&sample_entry("target-a", "completed", 100, 1))
            .unwrap();

        journal.truncate().unwrap();

        let entries = journal.replay().unwrap();
        let content = std::fs::read_to_string(journal.path()).unwrap();
        assert!(entries.is_empty());
        assert!(content.is_empty());
    }

    #[test]
    fn path_returns_underlying_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scan.log");
        let journal = WriteAheadJournal::new(path.clone());
        assert_eq!(journal.path(), path.as_path());
    }
}

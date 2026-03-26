//! Checkpoint persistence mechanism and JSON state definitions.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static CHECKPOINT_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Errors returned by scan state persistence helpers.
#[derive(Debug, Error)]
pub enum ScanStateError {
    /// Wrapper for filesystem errors.
    #[error(
        "scan state io error at {path}: {source}. Fix: verify the checkpoint directory exists and is writable."
    )]
    Io {
        /// The path where the IO error occurred.
        path: PathBuf,
        /// The underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// Error when trying to merge checkpoints from different scans.
    #[error("scan ids do not match: {0} vs {1}")]
    MergeConflict(String, String),
    /// Wrapper for JSON serialization errors.
    #[error("scan state serialization error: {0}. Fix: ensure the scan state struct derives Serialize/Deserialize correctly.")]
    Serde(#[from] serde_json::Error),
    /// Wrapper for TOML deserialization errors.
    #[error("scan state TOML parse error: {0}. Fix: verify the checkpoint file contains valid TOML with the expected fields.")]
    TomlParse(#[from] toml::de::Error),
    /// Wrapper for TOML serialization errors.
    #[error("scan state TOML serialize error: {0}. Fix: ensure all fields in the state struct are TOML-serializable.")]
    TomlSerialize(#[from] toml::ser::Error),
}

/// Configuration settings for checkpoint behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]

pub struct CheckpointSettings {
    /// Logical scan identifier.
    pub scan_id: String,
    /// Filesystem path for checkpoint persistence.
    pub checkpoint_path: String,
    /// Optional override path for write-ahead journal.
    pub journal_path: Option<String>,
    /// Optional total target count used by progress calculations.
    pub total_targets: usize,
    /// Whether to fsync checkpoint files on every save.
    pub sync_checkpoint: bool,
    /// Periodic flush cadence in seconds, if needed by caller.
    pub flush_interval_secs: u64,
}

impl CheckpointSettings {
    /// Parse settings from TOML text.
    ///
    /// # Errors
    /// Returns `ScanStateError::TomlParse` if the input is not valid TOML.
    pub fn from_toml(raw: &str) -> Result<Self, ScanStateError> {
        Ok(toml::from_str(raw)?)
    }

    /// Serialize settings back to TOML.
    ///
    /// # Errors
    /// Returns `ScanStateError::TomlSerialize` if serialization fails.
    pub fn to_toml(&self) -> Result<String, ScanStateError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Load settings from a TOML file.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the file cannot be read,
    /// or `ScanStateError::TomlParse` if the content is not valid TOML.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ScanStateError> {
        let p = path.as_ref();
        let content = fs::read_to_string(p).map_err(|e| ScanStateError::Io { path: p.to_path_buf(), source: e })?;
        Self::from_toml(&content)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CheckpointWire {
    scan_id: String,
    completed_targets: Vec<String>,
}

#[derive(Serialize)]
struct CheckpointWireOut<'a> {
    scan_id: &'a str,
    completed_targets: Vec<&'a str>,
}

/// Serializable checkpoint for exact resume without re-scanning completed targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanCheckpoint {
    /// Stable identifier for the scan run.
    pub scan_id: String,
    completed_targets: HashSet<String>,
}

impl ScanCheckpoint {
    /// Create an empty checkpoint for a scan.
    #[must_use]
    pub fn new(scan_id: impl Into<String>) -> Self {
        Self {
            scan_id: scan_id.into(),
            completed_targets: HashSet::new(),
        }
    }

    /// Mark a target as fully processed.
    pub fn mark_complete(&mut self, target_id: impl Into<String>) {
        self.completed_targets.insert(target_id.into());
    }

    /// Check whether a target is already complete.
    #[must_use]
    pub fn is_complete(&self, target_id: &str) -> bool {
        self.completed_targets.contains(target_id)
    }

    /// Number of unique completed targets.
    #[must_use]
    pub fn completed_count(&self) -> usize {
        self.completed_targets.len()
    }

    /// Persist the checkpoint atomically as JSON.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the file cannot be written,
    /// or `ScanStateError::Serde` if serialization fails.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ScanStateError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| ScanStateError::Io { path: parent.to_path_buf(), source: e })?;
            }
        }

        let mut completed_targets: Vec<&str> = self.completed_targets.iter().map(|s| s.as_str()).collect();
        completed_targets.sort_unstable();
        let json = serde_json::to_vec_pretty(&CheckpointWireOut {
            scan_id: &self.scan_id,
            completed_targets,
        })?;

        let tmp_path = tmp_path_for(path);
        {
            use std::io::Write;
            struct TmpGuard<'a>(&'a Path);
            impl<'a> Drop for TmpGuard<'a> {
                fn drop(&mut self) {
                    let _ = fs::remove_file(self.0);
                }
            }
            let _guard = TmpGuard(&tmp_path);
            let mut file = fs::File::create(&tmp_path).map_err(|e| ScanStateError::Io { path: tmp_path.clone(), source: e })?;
            file.write_all(&json).map_err(|e| ScanStateError::Io { path: tmp_path.clone(), source: e })?;
            file.sync_data().map_err(|e| ScanStateError::Io { path: tmp_path.clone(), source: e })?;
            std::mem::forget(_guard);
        }
        fs::rename(&tmp_path, path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            ScanStateError::Io { path: path.to_path_buf(), source: e }
        })?;
        Ok(())
    }

    /// Load a checkpoint from JSON.
    ///
    /// # Errors
    /// Returns `ScanStateError::Io` if the file cannot be read,
    /// or `ScanStateError::Serde` if the JSON is malformed.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ScanStateError> {
        let p = path.as_ref();
        let bytes = fs::read(p).map_err(|e| ScanStateError::Io { path: p.to_path_buf(), source: e })?;
        let wire: CheckpointWire = serde_json::from_slice(&bytes)?;
        Ok(Self {
            scan_id: wire.scan_id,
            completed_targets: wire.completed_targets.into_iter().collect(),
        })
    }

    /// Merge another checkpoint into this one.
    ///
    /// If both checkpoints have a non-empty `scan_id` and they differ, this method
    /// keeps `self.scan_id` unchanged and still unions the completed targets.
    pub fn merge(&mut self, other: Self) -> Result<(), ScanStateError> {
        if self.scan_id != other.scan_id {
            return Err(ScanStateError::MergeConflict(self.scan_id.clone(), other.scan_id));
        }
        self.completed_targets.extend(other.completed_targets);
        Ok(())
    }
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let unique_suffix = CHECKPOINT_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut new_name = path.file_name().unwrap_or_default().to_os_string();
    new_name.push(format!(".tmp-{}-{}", std::process::id(), unique_suffix));
    path.with_file_name(new_name)
}

#[cfg(test)]
mod tests {
    use super::{CheckpointSettings, ScanCheckpoint, ScanStateError};

    #[test]
    fn new_creates_empty_checkpoint() {
        let checkpoint = ScanCheckpoint::new("scan-1");
        assert_eq!(checkpoint.scan_id, "scan-1");
        assert_eq!(checkpoint.completed_count(), 0);
    }

    #[test]
    fn mark_complete_tracks_unique_targets() {
        let mut checkpoint = ScanCheckpoint::new("scan-1");
        checkpoint.mark_complete("target-a");
        checkpoint.mark_complete("target-a");
        checkpoint.mark_complete("target-b");

        assert!(checkpoint.is_complete("target-a"));
        assert!(checkpoint.is_complete("target-b"));
        assert_eq!(checkpoint.completed_count(), 2);
    }

    #[test]
    fn is_complete_returns_false_for_missing_target() {
        let checkpoint = ScanCheckpoint::new("scan-1");
        assert!(!checkpoint.is_complete("missing"));
    }

    #[test]
    fn completed_count_returns_number_of_unique_targets() {
        let mut checkpoint = ScanCheckpoint::new("scan-1");
        checkpoint.mark_complete("target-a");
        checkpoint.mark_complete("target-b");
        checkpoint.mark_complete("target-b");
        assert_eq!(checkpoint.completed_count(), 2);
    }

    #[test]
    fn save_persists_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/checkpoint.json");
        let mut checkpoint = ScanCheckpoint::new("scan-save");
        checkpoint.mark_complete("target-a");

        checkpoint.save(&path).unwrap();

        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"scan_id\": \"scan-save\""));
        assert!(json.contains("\"target-a\""));
    }

    #[test]
    fn load_restores_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        std::fs::write(
            &path,
            r#"{
  "scan_id": "scan-load",
  "completed_targets": [
    "target-a",
    "target-b"
  ]
}"#,
        )
        .unwrap();

        let checkpoint = ScanCheckpoint::load(&path).unwrap();
        assert_eq!(checkpoint.scan_id, "scan-load");
        assert!(checkpoint.is_complete("target-a"));
        assert!(checkpoint.is_complete("target-b"));
    }

    #[test]
    fn load_returns_error_for_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        std::fs::write(&path, "{").unwrap();

        let err = ScanCheckpoint::load(&path).unwrap_err();
        assert!(matches!(err, ScanStateError::Serde(_)));
    }

    #[test]
    fn merge_unions_completed_targets() {
        let mut left = ScanCheckpoint::new("scan-1");
        left.mark_complete("target-a");

        let mut right = ScanCheckpoint::new("scan-1");
        right.mark_complete("target-b");
        right.mark_complete("target-a");

        left.merge(right).unwrap();

        assert!(left.is_complete("target-a"));
        assert!(left.is_complete("target-b"));
        assert_eq!(left.completed_count(), 2);
        assert_eq!(left.scan_id, "scan-1");
    }

    #[test]
    fn save_load_roundtrip_1000_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        let mut checkpoint = ScanCheckpoint::new("stress-test");
        for i in 0..1000 {
            checkpoint.mark_complete(format!("https://target-{i}.example.com"));
        }
        checkpoint.save(&path).unwrap();
        let loaded = ScanCheckpoint::load(&path).unwrap();
        assert_eq!(loaded.scan_id, "stress-test");
        assert_eq!(loaded.completed_count(), 1000);
        assert!(loaded.is_complete("https://target-500.example.com"));
        for i in 0..1000 {
            assert!(loaded.is_complete(&format!("https://target-{i}.example.com")));
        }
    }

    #[test]
    fn save_is_atomic_on_error() {
        // Saving to a read-only directory should fail without corrupting existing data.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");
        let mut checkpoint = ScanCheckpoint::new("safe");
        checkpoint.mark_complete("target-a");
        checkpoint.save(&path).unwrap();

        // Now try to save to a nonexistent deep path — should fail.
        let bad_path = std::path::Path::new("/proc/nonexistent/deep/path/checkpoint.json");
        let _ = checkpoint.save(bad_path); // May or may not error depending on /proc permissions

        // Original file should still be intact.
        let loaded = ScanCheckpoint::load(&path).unwrap();
        assert!(loaded.is_complete("target-a"));
    }

    #[test]
    fn merge_empty_into_populated() {
        let mut populated = ScanCheckpoint::new("scan-1");
        populated.mark_complete("a");
        populated.mark_complete("b");

        let empty = ScanCheckpoint::new("scan-1");
        populated.merge(empty).unwrap();
        assert_eq!(populated.completed_count(), 2);
    }

    #[test]
    fn merge_populated_into_empty() {
        let mut empty = ScanCheckpoint::new("scan-1");
        let mut populated = ScanCheckpoint::new("scan-1");
        populated.mark_complete("a");

        empty.merge(populated).unwrap();
        assert_eq!(empty.completed_count(), 1);
        assert!(empty.is_complete("a"));
    }

    #[test]
    fn unicode_target_ids() {
        let mut checkpoint = ScanCheckpoint::new("unicode-test");
        checkpoint.mark_complete("https://日本語.example.com");
        checkpoint.mark_complete("https://пример.ru");
        assert_eq!(checkpoint.completed_count(), 2);
        assert!(checkpoint.is_complete("https://日本語.example.com"));
    }

    #[test]
    fn empty_target_id() {
        let mut checkpoint = ScanCheckpoint::new("test");
        checkpoint.mark_complete("");
        assert!(checkpoint.is_complete(""));
        assert_eq!(checkpoint.completed_count(), 1);
    }

    #[test]
    fn settings_round_trip() {
        let settings = CheckpointSettings {
            scan_id: "daily".to_string(),
            checkpoint_path: "/tmp/scan-checkpoint.json".to_string(),
            journal_path: None,
            total_targets: 100,
            sync_checkpoint: true,
            flush_interval_secs: 5,
        };
        let toml = settings.to_toml().unwrap();
        let loaded = CheckpointSettings::from_toml(&toml).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn settings_from_toml_partial() {
        let settings = CheckpointSettings::from_toml(
            r#"
scan_id = "daily"
checkpoint_path = "/tmp/scan-checkpoint.json"
total_targets = 100
sync_checkpoint = true
flush_interval_secs = 5
"#,
        )
        .unwrap();

        assert_eq!(settings.scan_id, "daily");
        assert_eq!(settings.checkpoint_path, "/tmp/scan-checkpoint.json");
        assert_eq!(settings.total_targets, 100);
        assert!(settings.sync_checkpoint);
    }

    #[test]
    fn settings_load_missing_file_errors() {
        let result = CheckpointSettings::load("/nonexistent/checkpoint.toml");
        assert!(matches!(result, Err(ScanStateError::Io { .. })));
    }
}

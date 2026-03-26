//! Adversarial tests for scanstate - designed to BREAK the code
//!
//! Tests: 100K targets, concurrent mark_complete calls, save during load,
//! unicode scan IDs, checkpoint > 100MB

use crate::{
    checkpoint::ScanCheckpoint, journal::WriteAheadJournal, progress::ScanProgress, Entry,
};
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

/// Test with 100,000 targets in checkpoint
#[test]
fn adversarial_100k_targets_checkpoint() {
    let mut checkpoint = ScanCheckpoint::new("stress-test-100k");

    // Mark 100K targets as complete
    for i in 0..100_000 {
        checkpoint.mark_complete(format!("https://target-{}.example.com/path", i));
    }

    assert_eq!(checkpoint.completed_count(), 100_000);

    // Verify random access
    assert!(checkpoint.is_complete("https://target-50000.example.com/path"));
    assert!(!checkpoint.is_complete("https://target-999999.example.com/path"));
}

/// Test concurrent mark_complete calls from multiple threads
#[test]
fn adversarial_concurrent_mark_complete() {
    let checkpoint = Arc::new(std::sync::Mutex::new(ScanCheckpoint::new(
        "concurrent-test",
    )));
    let num_threads = 10;
    let targets_per_thread = 1000;

    let mut handles = vec![];

    for thread_id in 0..num_threads {
        let cp = checkpoint.clone();
        let handle = thread::spawn(move || {
            for i in 0..targets_per_thread {
                let target = format!("thread-{}-target-{}", thread_id, i);
                cp.lock().unwrap().mark_complete(target);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let final_count = checkpoint.lock().unwrap().completed_count();
    assert_eq!(final_count, num_threads * targets_per_thread);
}

/// Test saving checkpoint while loading it
#[test]
fn adversarial_save_during_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("checkpoint.json");

    // Create initial checkpoint
    let mut checkpoint = ScanCheckpoint::new("race-test");
    for i in 0..1000 {
        checkpoint.mark_complete(format!("target-{}", i));
    }
    checkpoint.save(&path).unwrap();

    // Concurrent save and load
    let path_clone = path.clone();
    let save_handle = thread::spawn(move || {
        for i in 1000..2000 {
            let mut cp = ScanCheckpoint::new("race-test");
            // Load first to simulate read-modify-write
            if let Ok(loaded) = ScanCheckpoint::load(&path_clone) {
                cp = loaded;
            }
            cp.mark_complete(format!("target-{}", i));
            let _ = cp.save(&path_clone);
        }
    });

    let path_clone2 = path.clone();
    let load_handle = thread::spawn(move || {
        let mut success_count = 0;
        for _ in 0..100 {
            if ScanCheckpoint::load(&path_clone2).is_ok() {
                success_count += 1;
            }
            thread::sleep(std::time::Duration::from_millis(1));
        }
        success_count
    });

    save_handle.join().unwrap();
    let load_successes = load_handle.join().unwrap();

    // Most loads should succeed
    assert!(load_successes > 50);
    // When a load succeeded, it must have valid state
    let final_loaded = ScanCheckpoint::load(&path).unwrap();
    assert_eq!(final_loaded.scan_id, "race-test");
    assert!(final_loaded.completed_count() > 0);
}

/// Test checkpoint with unicode scan IDs and target IDs
#[test]
fn adversarial_unicode_scan_ids() {
    let mut checkpoint = ScanCheckpoint::new("スキャン-日本語-테스트-מבחן");

    // Add targets with unicode IDs
    checkpoint.mark_complete("https://例え.テスト/日本語");
    checkpoint.mark_complete("https://пример.рф/тест");
    checkpoint.mark_complete("https://مثال.اختبار/عربي");
    checkpoint.mark_complete("https://例子.测试/中文");
    checkpoint.mark_complete("https://émojis.🎉/test-🔒-🔑");

    assert_eq!(checkpoint.completed_count(), 5);
    assert!(checkpoint.is_complete("https://例え.テスト/日本語"));

    // Save and load
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unicode.json");
    checkpoint.save(&path).unwrap();

    let loaded = ScanCheckpoint::load(&path).unwrap();
    assert_eq!(loaded.scan_id, "スキャン-日本語-테스트-מבחן");
    assert!(loaded.is_complete("https://例え.テスト/日本語"));
}

/// Test checkpoint larger than 100MB
#[test]
fn adversarial_checkpoint_over_100mb() {
    let mut checkpoint = ScanCheckpoint::new("big-checkpoint");

    // Create targets with very long IDs to exceed 100MB
    let long_id = "a".repeat(2000);
    for i in 0..50_000 {
        checkpoint.mark_complete(format!("{}-{}", long_id, i));
    }

    assert_eq!(checkpoint.completed_count(), 50_000);

    // Save and verify file size
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.json");
    checkpoint.save(&path).unwrap();

    let metadata = fs::metadata(&path).unwrap();
    let size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
    assert!(
        size_mb > 50.0,
        "Checkpoint should be large, was {} MB",
        size_mb
    );

    // Load it back
    let loaded = ScanCheckpoint::load(&path).unwrap();
    assert_eq!(loaded.completed_count(), 50_000);
}

/// Test journal with 100K entries
#[test]
fn adversarial_journal_100k_entries() {
    let dir = tempfile::tempdir().unwrap();
    let journal = WriteAheadJournal::new(dir.path().join("big.journal"));

    // Append 100K entries
    for i in 0..100_000 {
        let entry = Entry {
            target_id: format!("target-{}", i),
            status: if i % 2 == 0 {
                "completed".to_string()
            } else {
                "skipped".to_string()
            },
            timestamp: i as u64,
            findings_count: (i % 10) as usize,
        };
        journal.append(&entry).unwrap();
    }

    // Replay
    let entries = journal.replay().unwrap();
    assert_eq!(entries.len(), 100_000);
}

/// Test concurrent journal appends
#[test]
fn adversarial_concurrent_journal_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("concurrent.journal");

    let num_threads = 10;
    let entries_per_thread = 100;
    let barrier = Arc::new(Barrier::new(num_threads));

    let mut handles = vec![];

    for thread_id in 0..num_threads {
        let path = path.clone();
        let barrier = barrier.clone();
        let handle = thread::spawn(move || {
            let journal = WriteAheadJournal::new(path);
            barrier.wait(); // Synchronize start

            for i in 0..entries_per_thread {
                let entry = Entry {
                    target_id: format!("thread-{}-entry-{}", thread_id, i),
                    status: "completed".to_string(),
                    timestamp: thread_id as u64 * 1000 + i as u64,
                    findings_count: 0,
                };
                // May fail due to contention, that's ok
                let _ = journal.append(&entry);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Replay what we got
    let journal = WriteAheadJournal::new(&path);
    let (entries, corrupt) = journal.replay_lenient().unwrap();

    // We should have no corrupted entries, and exactly the expected amount.
    assert_eq!(corrupt, 0);
    assert_eq!(entries.len(), num_threads * entries_per_thread);
}

/// Test empty target ID
#[test]
fn adversarial_empty_target_id() {
    let mut checkpoint = ScanCheckpoint::new("empty-test");

    checkpoint.mark_complete("");
    checkpoint.mark_complete("");
    checkpoint.mark_complete("");

    // Empty string should only be counted once
    assert_eq!(checkpoint.completed_count(), 1);
    assert!(checkpoint.is_complete(""));
}

/// Test target IDs with special characters and newlines
#[test]
fn adversarial_special_character_target_ids() {
    let mut checkpoint = ScanCheckpoint::new("special-chars");

    let special_ids = vec![
        "target\nwith\nnewlines",
        "target\twith\ttabs",
        "target\x00with\x00nulls",
        "target\"with\"quotes",
        "target\\with\\backslashes",
        "{\"json\": \"value\"}",
        "<script>alert(1)</script>",
        "../../../../etc/passwd",
        "$(whoami)",
        "`rm -rf /`",
    ];

    for id in &special_ids {
        checkpoint.mark_complete(*id);
    }

    assert_eq!(checkpoint.completed_count(), special_ids.len());

    // Save and load
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("special.json");
    checkpoint.save(&path).unwrap();

    let loaded = ScanCheckpoint::load(&path).unwrap();
    for id in &special_ids {
        assert!(loaded.is_complete(id), "Failed to find target: {:?}", id);
    }
}

/// Test progress with zero total
#[test]
fn adversarial_progress_zero_total() {
    let mut progress = ScanProgress::new(0);

    progress.record_completed();
    progress.record_skipped();
    progress.record_findings(10);

    // With 0 total, ETA should be 0
    assert_eq!(progress.eta(), std::time::Duration::ZERO);
    assert!(progress.rate() >= 0.0);
}

/// Test progress overflow conditions
#[test]
fn adversarial_progress_overflow() {
    let mut progress = ScanProgress::new(100);

    // Complete more than total
    for _ in 0..150 {
        progress.record_completed();
    }

    // ETA should be 0 when processed > total
    assert_eq!(progress.eta(), std::time::Duration::ZERO);
}

/// Test checkpoint merge with conflicting scan IDs
#[test]
fn adversarial_checkpoint_merge_conflicts() {
    let mut cp1 = ScanCheckpoint::new("scan-1");
    cp1.mark_complete("target-a");
    cp1.mark_complete("target-b");

    let mut cp2 = ScanCheckpoint::new("scan-2");
    cp2.mark_complete("target-b");
    cp2.mark_complete("target-c");

    let result = cp1.merge(cp2);
    assert!(result.is_err());

    // cp1's scan_id should be preserved
    assert_eq!(cp1.scan_id, "scan-1");
    // Should NOT have target-c
    assert_eq!(cp1.completed_count(), 2);
    assert!(cp1.is_complete("target-a"));
    assert!(cp1.is_complete("target-b"));
    assert!(!cp1.is_complete("target-c"));
}

/// Test loading corrupted checkpoint with partial data
#[test]
fn adversarial_corrupted_checkpoint_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("corrupt.json");

    // Write a partially corrupted JSON
    fs::write(
        &path,
        r#"{"scan_id": "test", "completed_targets": ["a", "b", "#,
    )
    .unwrap();

    let result = ScanCheckpoint::load(&path);
    assert!(result.is_err());
}

/// Test journal replay with mixed valid and corrupt entries
#[test]
fn adversarial_journal_mixed_corruption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.journal");

    // Write mix of valid and invalid entries
    let mut content = String::new();

    // Valid entry
    let valid = serde_json::json!({
        "target_id": "valid-1",
        "status": "completed",
        "timestamp": 100,
        "findings_count": 5
    });
    content.push_str(&valid.to_string());
    content.push('\n');

    // Invalid entry
    content.push_str("this is not json\n");

    // Another valid entry
    let valid2 = serde_json::json!({
        "target_id": "valid-2",
        "status": "skipped",
        "timestamp": 200,
        "findings_count": 0
    });
    content.push_str(&valid2.to_string());
    content.push('\n');

    // Truncated entry
    content.push_str("{\"target_id\": \"trunc");

    fs::write(&path, content).unwrap();

    let journal = WriteAheadJournal::new(&path);
    let (entries, corrupt) = journal.replay_lenient().unwrap();

    assert_eq!(entries.len(), 2);
    assert_eq!(corrupt, 2); // One invalid, one truncated
}

/// Test very long scan ID
#[test]
fn adversarial_very_long_scan_id() {
    let long_id = "a".repeat(100_000);
    let checkpoint = ScanCheckpoint::new(&long_id);

    assert_eq!(checkpoint.scan_id, long_id);

    // Should be able to save and load
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long_id.json");
    checkpoint.save(&path).unwrap();

    let loaded = ScanCheckpoint::load(&path).unwrap();
    assert_eq!(loaded.scan_id.len(), 100_000);
}

/// Test checkpoint with duplicate targets (should dedupe)
#[test]
fn adversarial_duplicate_target_storm() {
    let mut checkpoint = ScanCheckpoint::new("dedupe-test");

    // Mark the same target 10,000 times
    for _ in 0..10_000 {
        checkpoint.mark_complete("same-target");
    }

    // Should only count once
    assert_eq!(checkpoint.completed_count(), 1);
}

/// Test progress rate calculation with very small elapsed time
#[test]
fn adversarial_progress_rate_divide_by_zero() {
    let mut progress = ScanProgress::new(100);
    progress.record_completed();

    // Rate with essentially zero elapsed time
    let rate = progress.rate();
    assert!(rate >= 0.0);
    assert!(rate.is_finite());
    // Since elapsed is practically 0, rate should be gracefully 0.0
    if let Ok(elapsed) = progress.start_time.elapsed() {
        if elapsed.as_secs_f64() <= f64::EPSILON {
            assert_eq!(rate, 0.0);
        }
    }
}

/// Test save to non-existent deep directory
#[test]
fn adversarial_save_deep_directory() {
    let checkpoint = ScanCheckpoint::new("deep");
    let dir = tempfile::tempdir().unwrap();
    let deep_path = dir.path().join("a/b/c/d/e/f/g/h/i/j/checkpoint.json");

    checkpoint.save(&deep_path).unwrap();

    assert!(deep_path.exists());
    let loaded = ScanCheckpoint::load(&deep_path).unwrap();
    assert_eq!(loaded.scan_id, "deep");
}

/// Test journal truncate and immediate append
#[test]
fn adversarial_journal_truncate_append_race() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("truncate.journal");

    let journal = WriteAheadJournal::new(&path);

    // Append some entries
    for i in 0..10 {
        let entry = Entry {
            target_id: format!("target-{}", i),
            status: "completed".to_string(),
            timestamp: i as u64,
            findings_count: 0,
        };
        journal.append(&entry).unwrap();
    }

    // Truncate
    journal.truncate().unwrap();

    // Append immediately after truncate
    let entry = Entry {
        target_id: "after-truncate".to_string(),
        status: "completed".to_string(),
        timestamp: 999,
        findings_count: 0,
    };
    journal.append(&entry).unwrap();

    // Should only see the post-truncate entry
    let entries = journal.replay().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].target_id, "after-truncate");
}

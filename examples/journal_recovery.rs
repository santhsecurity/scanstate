//! Example demonstrating write-ahead journal for crash recovery.
//!
//! Run: cargo run --example journal_recovery

use scanstate::{Entry, WriteAheadJournal};

fn main() {
    let dir = tempfile::tempdir().unwrap();
    let journal_path = dir.path().join("scan.journal");

    // Simulating a scanner writing progress to a journal
    let journal = WriteAheadJournal::new(&journal_path);
    journal
        .append(&Entry {
            target_id: "https://example.com/api/v1".into(),
            status: "completed".into(),
            timestamp: 1690000000,
            findings_count: 1,
        })
        .unwrap();

    println!("Appended entry to journal.");

    // Simulating recovery after a crash
    let entries = journal.replay().unwrap();
    println!("Recovered {} entries from journal.", entries.len());
    for entry in entries {
        println!("  Target: {}, Status: {}", entry.target_id, entry.status);
    }
}

# TOKIO-LEVEL Deep Audit: scanstate v0.1.1

**Audit Date:** 2026-03-26  
**Scope:** Single-crate analysis of checkpoint persistence, journal recovery, TOML serialization, thread safety, and corruption handling.  
**Auditor:** Automated analysis + manual code review

---

## Executive Summary

| Component | Status | Risk Level |
|-----------|--------|------------|
| Checkpoint Atomicity | ✅ SOUND | Low |
| Journal Recovery | ⚠️ PARTIAL | Medium |
| TOML Serialization | ⚠️ LIMITED | Low |
| Thread Safety | ❌ UNSOUND | **High** |
| Corruption Handling | ⚠️ PARTIAL | Medium |

**Critical Finding:** The crate is **NOT Tokio-safe** for concurrent checkpoint writes. Multiple async tasks writing checkpoints will cause data races and potential corruption.

---

## 1. Checkpoint Write Atomicity

### Question: Is checkpoint write atomic? What happens on crash mid-save?

### Analysis

The `ScanCheckpoint::save()` implementation (lines 139-174 in `src/checkpoint.rs`) uses a **write-to-temp + atomic rename** pattern:

```rust
pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ScanStateError> {
    // 1. Create parent directories
    // 2. Serialize to JSON
    // 3. Generate unique temp path with atomic counter + PID
    let tmp_path = tmp_path_for(path);
    
    // 4. Write to temp file with cleanup guard
    let _guard = TmpGuard(&tmp_path);
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(&json)?;
    file.sync_data()?;  // <-- fsync before rename
    std::mem::forget(_guard);  // Prevent cleanup on success
    
    // 5. Atomic rename
    fs::rename(&tmp_path, path)?;
    Ok(())
}
```

### Crash Scenarios

| Crash Timing | Outcome | Data Integrity |
|--------------|---------|----------------|
| Before `write_all()` | Temp file incomplete or absent | Original checkpoint intact |
| During `write_all()` | Temp file partially written | Original checkpoint intact |
| After `write_all()`, before `sync_data()` | Temp file complete but unflushed | Original checkpoint intact (journal may recover) |
| After `sync_data()`, before `rename()` | Temp file complete and flushed | Original checkpoint intact |
| During `rename()` | **Depends on filesystem** | Atomic rename guarantees file consistency |
| After `rename()` | Checkpoint fully saved | New state persisted |

### Verdict: ✅ **SOUND**

The implementation correctly implements atomic writes:
- **TmpGuard** ensures cleanup on panic/unwind (RAII pattern)
- **`sync_data()`** ensures data reaches stable storage before rename
- **`fs::rename()`** is atomic on POSIX (same filesystem) and Windows
- **Unique temp paths** prevent collisions between processes/threads

### Limitations

```rust
// The temp counter uses Relaxed ordering - sufficient for uniqueness
// but doesn't provide happens-before relationships
static CHECKPOINT_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
```

`Ordering::Relaxed` is acceptable here since uniqueness only requires monotonic increments within a process, not cross-thread synchronization.

---

## 2. Journal Recovery After Interrupted Scans

### Question: Does journal recovery work after interrupted scans?

### Analysis

The `WriteAheadJournal` (lines 23-148 in `src/journal.rs`) implements newline-delimited JSON with two replay modes:

### Strict Mode (`replay()`)
```rust
pub fn replay(&self) -> Result<Vec<Entry>, ScanStateError> {
    // Fails fast on first corrupt entry
    entries.push(serde_json::from_slice(&buf)?);
}
```

### Lenient Mode (`replay_lenient()`)
```rust
pub fn replay_lenient(&self) -> Result<(Vec<Entry>, usize), ScanStateError> {
    // Skips corrupt entries, counts them
    match serde_json::from_slice::<Entry>(&buf) {
        Ok(entry) => entries.push(entry),
        Err(_) => corrupt_count += 1,
    }
}
```

### Recovery Scenarios

| Scenario | `replay()` | `replay_lenient()` |
|----------|------------|---------------------|
| Clean shutdown | ✅ All entries | ✅ All entries |
| Crash mid-append (incomplete line) | ❌ Fails | ✅ Skips partial line |
| Corrupt entry in middle | ❌ Fails | ✅ Recovers before/after |
| Truncated file (mid-line) | ❌ Fails | ⚠️ Counts as corrupt |
| Empty file | ✅ Empty vec | ✅ Empty vec |
| Missing file | ✅ Empty vec | ✅ Empty vec |
| Whitespace lines | ✅ Skipped | ✅ Skipped |

### Critical Gaps

```rust
// Line 52-55: Each append fsyncs - GOOD for durability
pub fn append(&self, entry: &Entry) -> Result<(), ScanStateError> {
    file.write_all(&buf)?;
    file.sync_all()?;  // Full fsync every entry
    Ok(())
}
```

**Performance Issue:** `sync_all()` on every append creates significant I/O overhead. For high-throughput scanners, this is a bottleneck.

**No Checkpoint Integration:** The journal and checkpoint are separate systems. There's no built-in coordination for:
- Checkpoint rotation after journal truncation
- Determining which journal entries are already reflected in checkpoint
- Replay-and-checkpoint workflow

### Verdict: ⚠️ **PARTIAL**

Recovery works for individual crash scenarios, but:
1. No built-in integration between checkpoint and journal
2. `replay()` fails fast without recovery options
3. Users must manually coordinate truncation with checkpointing

### Recommended Pattern

```rust
// NOT PROVIDED BY CRATE - users must implement:
fn recover_from_crash(journal_path: &Path, checkpoint_path: &Path) -> Result<ScanCheckpoint> {
    let journal = WriteAheadJournal::new(journal_path);
    let (entries, corrupt) = journal.replay_lenient()?;
    
    let mut checkpoint = ScanCheckpoint::load(checkpoint_path)?;
    for entry in entries {
        if entry.status == "completed" {
            checkpoint.mark_complete(&entry.target_id);
        }
    }
    checkpoint.save(checkpoint_path)?;
    journal.truncate()?;
    Ok(checkpoint)
}
```

---

## 3. TOML Serialization Correctness

### Question: Is the TOML serialization correct for all types?

### Analysis

TOML support is **limited to `CheckpointSettings`** only:

```rust
// src/checkpoint.rs lines 40-85
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointSettings {
    pub scan_id: String,
    pub checkpoint_path: String,
    pub journal_path: Option<String>,
    pub total_targets: usize,
    pub sync_checkpoint: bool,
    pub flush_interval_secs: u64,
}
```

**Key Finding:** `ScanCheckpoint` (the main state) does **NOT** support TOML - only JSON.

### Type Compatibility Matrix

| Type | TOML Serialize | TOML Deserialize | Notes |
|------|---------------|------------------|-------|
| `CheckpointSettings` | ✅ | ✅ | All standard types |
| `ScanCheckpoint` | ❌ N/A | ❌ N/A | JSON only |
| `Entry` | ❌ N/A | ❌ N/A | JSON only |
| `ScanProgress` | ❌ N/A | ❌ N/A | JSON only |

### TOML Test Coverage

```rust
// Lines 388-425: Settings round-trip tests
#[test]
fn settings_round_trip() { /* ... */ }

#[test]
fn settings_from_toml_partial() { /* ... */ }
```

### Verdict: ⚠️ **LIMITED**

TOML serialization is **correct but narrow**:
- Only `CheckpointSettings` supports TOML
- No datetime types (common TOML pitfall)
- No heterogeneous arrays
- All types used are TOML-compatible (strings, numbers, bools)

### Gap: SystemTime Serialization

```rust
// src/progress.rs line 18
pub struct ScanProgress {
    pub start_time: SystemTime,  // Uses serde's SystemTime serialization
}
```

`SystemTime` serializes as a `(secs, nanos)` tuple in JSON. If TOML support were added for `ScanProgress`, this would need special handling since TOML datetime format differs.

---

## 4. Thread Safety Under Concurrent Checkpoint Writes

### Question: Is the crate thread-safe for concurrent checkpoint writes?

### Analysis

### Type Thread-Safety

| Type | `Send` | `Sync` | Internal Sync |
|------|--------|--------|---------------|
| `ScanCheckpoint` | ✅ | ✅ | ❌ **None** |
| `WriteAheadJournal` | ✅ | ✅ | ❌ **None** |
| `ScanProgress` | ✅ | ✅ | ❌ **None** |
| `CheckpointSettings` | ✅ | ✅ | N/A (immutable) |

### Critical Finding: NO INTERNAL SYNCHRONIZATION

```rust
// src/checkpoint.rs lines 100-105
pub struct ScanCheckpoint {
    pub scan_id: String,
    completed_targets: HashSet<String>,  // Standard HashSet - NOT thread-safe
}

// mark_complete takes &mut self - but no locking
pub fn mark_complete(&mut self, target_id: impl Into<String>) {
    self.completed_targets.insert(target_id.into());  // Race condition!
}
```

### Concurrent Write Scenarios

```rust
// DANGER: This pattern will CORRUPT data
let checkpoint = Arc::new(ScanCheckpoint::new("scan-1"));

// Spawn 10 tokio tasks, all writing
for i in 0..10 {
    let cp = checkpoint.clone();
    tokio::spawn(async move {
        // RACE CONDITION: Multiple concurrent writes
        cp.save(format!("checkpoint-{}.json", i)).unwrap();
    });
}
```

**Problems:**
1. `ScanCheckpoint` has no interior mutability - requires `&mut self` for updates
2. `save()` reads from `&self` but filesystem operations race
3. Temp file counter could collide under extreme contention
4. `WriteAheadJournal::append()` opens/closes file on each call - no file locking

### What The Adversarial Tests Show

```rust
// src/adversarial_tests.rs lines 32-58
fn adversarial_concurrent_mark_complete() {
    let checkpoint = Arc::new(std::sync::Mutex::new(ScanCheckpoint::new(...)));
    // ...
}
```

The adversarial tests **externally synchronize** with `std::sync::Mutex`. This proves the authors knew the types aren't thread-safe.

### Journal Concurrent Append

```rust
// src/adversarial_tests.rs lines 193-236
fn adversarial_concurrent_journal_append() {
    // Creates separate journal handles per thread
    let journal = WriteAheadJournal::new(path);
    // All threads append simultaneously - relies on OS file locking
}
```

**Result:** Test passes on Linux (OS-level append atomicity), but this is **NOT** guaranteed by the crate and may fail on other platforms or network filesystems.

### Verdict: ❌ **UNSOUND**

The crate is **NOT safe for concurrent use** without external synchronization:

| Scenario | Safe? | Required Mitigation |
|----------|-------|---------------------|
| Multiple threads, one checkpoint | ❌ No | External `Mutex<ScanCheckpoint>` |
| Multiple async tasks, one journal | ⚠️ Risky | External `Mutex<WriteAheadJournal>` |
| One writer, multiple readers | ✅ Yes | Atomic rename provides visibility |
| Concurrent checkpoint + journal | ⚠️ Risky | Coordinated locking required |

### Tokio-Specific Concerns

```rust
// THIS IS UNSAFE - DO NOT USE
async fn update_checkpoint(cp: &ScanCheckpoint) {
    // Tokio may move between threads between await points
    cp.save("checkpoint.json").await;  // If this were async...
}
```

Currently all I/O is **blocking** (`std::fs`), so in Tokio you must use `spawn_blocking`:

```rust
// Correct Tokio usage
let cp = Arc::new(Mutex::new(checkpoint));
tokio::task::spawn_blocking(move || {
    cp.lock().unwrap().save("checkpoint.json")
}).await??;
```

### Recommendations

1. **Document** that types are `Send` but not internally synchronized
2. **Provide** `tokio::sync::RwLock` wrapper example in docs
3. **Consider** adding `SyncCheckpoint` wrapper type with internal locking

---

## 5. Corrupt/Truncated Checkpoint Files

### Question: What happens with corrupt/truncated checkpoint files?

### Analysis

### `ScanCheckpoint::load()` Behavior

```rust
// src/checkpoint.rs lines 181-189
pub fn load(path: impl AsRef<Path>) -> Result<Self, ScanStateError> {
    let bytes = fs::read(p)?;
    let wire: CheckpointWire = serde_json::from_slice(&bytes)?;  // Fails on corrupt
    Ok(Self { /* ... */ })
}
```

### Corruption Scenarios

| Scenario | `load()` Result | Recoverable? |
|----------|-----------------|--------------|
| Valid JSON | ✅ Ok(checkpoint) | N/A |
| Invalid JSON syntax | ❌ `ScanStateError::Serde` | No built-in recovery |
| Truncated JSON | ❌ `ScanStateError::Serde` | No built-in recovery |
| Wrong JSON structure | ❌ `ScanStateError::Serde` | No built-in recovery |
| Empty file | ❌ `ScanStateError::Serde` | No |
| File permissions | ❌ `ScanStateError::Io` | Fix permissions |
| Missing file | ❌ `ScanStateError::Io` | Use `load_or_new()` |

### Comparison: Checkpoint vs Journal

| Feature | Checkpoint | Journal |
|---------|-----------|---------|
| Strict load | ✅ `load()` | ✅ `replay()` |
| Lenient load | ❌ **Not implemented** | ✅ `replay_lenient()` |
| Partial recovery | ❌ None | ✅ Skips corrupt lines |
| Corruption detection | ❌ JSON parse fails | ✅ Counts corrupt entries |

### Gap: No Lenient Checkpoint Loading

Unlike the journal, checkpoint has **no recovery mode**:

```rust
// Journal has this - Checkpoint does NOT
pub fn replay_lenient(&self) -> Result<(Vec<Entry>, usize), ScanStateError>;

// Would be useful for checkpoints:
pub fn load_lenient(&self) -> Result<(ScanCheckpoint, Vec<CorruptTarget>), ScanStateError>;
```

### Adversarial Test Coverage

```rust
// src/adversarial_tests.rs lines 339-353
fn adversarial_corrupted_checkpoint_recovery() {
    fs::write(&path, r#"{"scan_id": "test", "completed_targets": ["a", "b", "#);
    let result = ScanCheckpoint::load(&path);
    assert!(result.is_err());  // Just confirms it fails - no recovery
}
```

### Verdict: ⚠️ **PARTIAL**

- **Journal:** Robust corruption handling with lenient mode
- **Checkpoint:** All-or-nothing - single corrupt byte loses entire checkpoint

### Workaround for Users

```rust
// Users must implement checkpoint backup/rotation
fn load_checkpoint_with_backup(path: &Path) -> Result<ScanCheckpoint> {
    match ScanCheckpoint::load(path) {
        Ok(cp) => Ok(cp),
        Err(_) => {
            // Try backup
            ScanCheckpoint::load(path.with_extension("json.bak"))
        }
    }
}
```

---

## Summary Table

| Concern | Implementation | Risk |
|---------|---------------|------|
| Atomic write | Write-temp + fsync + rename | ✅ Low |
| Crash recovery | Temp guard + atomic rename | ✅ Low |
| Journal durability | Per-entry fsync | ✅ Correct but slow |
| Journal recovery | Strict + lenient modes | ⚠️ Partial |
| TOML support | Settings only | ⚠️ Limited |
| Thread safety | None (external lock required) | ❌ **High** |
| Corrupt checkpoint | Total failure | ⚠️ Medium |
| Corrupt journal | Lenient recovery | ✅ Good |

---

## Recommendations

### Immediate (Before Production Use)

1. **Wrap all checkpoint operations in `tokio::sync::RwLock` or `std::sync::Mutex`**
2. **Keep checkpoint backups** - no built-in recovery for corrupt checkpoints
3. **Use `replay_lenient()`** for journal recovery, not strict `replay()`

### Crate Improvements

1. Add `tokio` feature with async I/O and internal synchronization
2. Implement `load_lenient()` for checkpoint partial recovery
3. Add checkpoint/journal integration helpers
4. Document thread-safety requirements clearly
5. Consider batch journal writes with periodic fsync for performance

---

## Appendix: Code Paths

| Function | File | Lines | Key Behavior |
|----------|------|-------|--------------|
| `ScanCheckpoint::save()` | `checkpoint.rs` | 139-174 | Atomic write via temp file |
| `ScanCheckpoint::load()` | `checkpoint.rs` | 181-189 | Strict JSON parse |
| `WriteAheadJournal::append()` | `journal.rs` | 40-57 | Per-entry fsync |
| `WriteAheadJournal::replay()` | `journal.rs` | 67-87 | Strict parse |
| `WriteAheadJournal::replay_lenient()` | `journal.rs` | 97-121 | Skip corrupt entries |
| `tmp_path_for()` | `checkpoint.rs` | 204-209 | Atomic counter + PID |

---

*End of Audit Report*

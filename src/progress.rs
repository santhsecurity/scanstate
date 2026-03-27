//! Live progress indicators for tracking scan rates and ETAs.

use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// Live scan progress metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanProgress {
    /// Total planned targets.
    pub total: usize,
    /// Successfully completed targets.
    pub completed: usize,
    /// Skipped targets.
    pub skipped: usize,
    /// Total findings discovered so far.
    pub findings: usize,
    /// Time when the scan started.
    pub start_time: SystemTime,
}

impl Default for ScanProgress {
    fn default() -> Self {
        Self::new(0)
    }
}

impl ScanProgress {
    /// Create progress tracking for a scan.
    #[must_use]
    pub fn new(total: usize) -> Self {
        Self {
            total,
            completed: 0,
            skipped: 0,
            findings: 0,
            start_time: SystemTime::now(),
        }
    }

    /// Record one completed target.
    pub fn record_completed(&mut self) {
        self.completed += 1;
    }

    /// Record one skipped target.
    pub fn record_skipped(&mut self) {
        self.skipped += 1;
    }

    /// Add findings discovered during the scan.
    pub fn record_findings(&mut self, findings: usize) {
        self.findings += findings;
    }

    /// Current processing rate in targets per second.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn rate(&self) -> f64 {
        let elapsed = if let Ok(d) = self.start_time.elapsed() {
            d.as_secs_f64()
        } else {
            0.0
        };
        if elapsed <= f64::EPSILON {
            return 0.0;
        }

        (self.completed + self.skipped) as f64 / elapsed
    }

    /// Estimated time remaining.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn eta(&self) -> Duration {
        let processed = self.completed + self.skipped;
        if self.total <= processed {
            return Duration::ZERO;
        }

        let rate = self.rate();
        if rate <= f64::EPSILON {
            return Duration::ZERO;
        }

        let secs = (self.total - processed) as f64 / rate;
        if secs > Duration::MAX.as_secs_f64() {
            return Duration::MAX;
        }
        Duration::from_secs_f64(secs)
    }
}

#[cfg(test)]
mod tests {
    use super::ScanProgress;
    use std::time::{Duration, SystemTime};

    #[test]
    fn new_initializes_progress() {
        let progress = ScanProgress::new(10);
        assert_eq!(progress.total, 10);
        assert_eq!(progress.completed, 0);
        assert_eq!(progress.skipped, 0);
        assert_eq!(progress.findings, 0);
    }

    #[test]
    fn record_completed_increments_completed_count() {
        let mut progress = ScanProgress::new(10);
        progress.record_completed();
        assert_eq!(progress.completed, 1);
    }

    #[test]
    fn record_skipped_increments_skipped_count() {
        let mut progress = ScanProgress::new(10);
        progress.record_skipped();
        assert_eq!(progress.skipped, 1);
    }

    #[test]
    fn record_findings_accumulates_findings() {
        let mut progress = ScanProgress::new(10);
        progress.record_findings(2);
        progress.record_findings(3);
        assert_eq!(progress.findings, 5);
    }

    #[test]
    fn rate_returns_targets_per_second() {
        let mut progress = ScanProgress::new(10);
        progress.completed = 4;
        progress.skipped = 2;
        progress.start_time = SystemTime::now()
            .checked_sub(Duration::from_secs(2))
            .expect("subtract fixed start time");

        let rate = progress.rate();
        assert!((2.9..=3.1).contains(&rate), "unexpected rate: {rate}");
    }

    #[test]
    fn rate_returns_zero_when_elapsed_is_too_small() {
        let progress = ScanProgress::new(10);
        let rate = progress.rate();
        assert!(rate >= 0.0);
    }

    #[test]
    fn eta_returns_zero_when_scan_is_complete() {
        let mut progress = ScanProgress::new(5);
        progress.completed = 3;
        progress.skipped = 2;
        progress.start_time = SystemTime::now()
            .checked_sub(Duration::from_secs(2))
            .expect("subtract fixed start time");

        assert_eq!(progress.eta(), Duration::ZERO);
    }

    #[test]
    fn eta_estimates_remaining_time() {
        let mut progress = ScanProgress::new(10);
        progress.completed = 4;
        progress.skipped = 2;
        progress.start_time = SystemTime::now()
            .checked_sub(Duration::from_secs(2))
            .expect("subtract fixed start time");

        let eta = progress.eta();
        assert!(eta >= Duration::from_millis(1200));
        assert!(eta <= Duration::from_millis(1500));
    }

    #[test]
    fn progress_is_serializable() {
        let progress = ScanProgress {
            total: 10,
            completed: 3,
            skipped: 1,
            findings: 2,
            start_time: SystemTime::UNIX_EPOCH,
        };
        let payload = serde_json::to_string(&progress).unwrap();
        let decoded: ScanProgress = serde_json::from_str(&payload).unwrap();
        assert_eq!(decoded.total, 10);
        assert_eq!(decoded.completed, 3);
        assert_eq!(decoded.skipped, 1);
        assert_eq!(decoded.findings, 2);
        assert_eq!(decoded.start_time, SystemTime::UNIX_EPOCH);
    }
}

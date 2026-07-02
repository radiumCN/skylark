//! Persistent daily traffic statistics.
//!
//! The frontend already derives per-second up/down deltas from the core's cumulative
//! counters. It periodically flushes accumulated bytes here via `cmd_add_traffic_sample`;
//! we bucket them by calendar day and persist to `traffic_stats.json` so totals survive
//! restarts (the in-session counters reset every proxy restart by design).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// One calendar day's transferred bytes.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct DailyTraffic {
    pub upload: u64,
    pub download: u64,
}

/// A day's entry flattened for the frontend (`date` = `YYYY-MM-DD`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrafficDay {
    pub date: String,
    pub upload: u64,
    pub download: u64,
}

/// On-disk shape: day-keyed map kept sorted by the BTreeMap key (ISO date sorts
/// chronologically). Wrapped in a struct so the file can gain fields later.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatsData {
    #[serde(default)]
    pub days: BTreeMap<String, DailyTraffic>,
}

/// Keep at most this many days of history; older buckets are pruned on each write.
const MAX_DAYS: usize = 180;

impl StatsData {
    /// Add a sample to the given day's bucket and prune history to `MAX_DAYS`.
    /// Pure (no I/O, no clock) so it is unit-testable; the caller supplies `date`.
    pub fn add_sample(&mut self, date: &str, upload: u64, download: u64) {
        let entry = self.days.entry(date.to_string()).or_default();
        entry.upload = entry.upload.saturating_add(upload);
        entry.download = entry.download.saturating_add(download);
        self.prune();
    }

    /// Drop the oldest entries beyond `MAX_DAYS`. BTreeMap iterates in key (date) order,
    /// so the first keys are the oldest.
    fn prune(&mut self) {
        while self.days.len() > MAX_DAYS {
            if let Some(oldest) = self.days.keys().next().cloned() {
                self.days.remove(&oldest);
            } else {
                break;
            }
        }
    }

    /// Most recent `days` entries, oldest-first. `days == 0` returns all.
    pub fn recent(&self, days: usize) -> Vec<TrafficDay> {
        let all: Vec<TrafficDay> = self
            .days
            .iter()
            .map(|(date, t)| TrafficDay {
                date: date.clone(),
                upload: t.upload,
                download: t.download,
            })
            .collect();
        if days == 0 || all.len() <= days {
            all
        } else {
            all[all.len() - days..].to_vec()
        }
    }
}

fn stats_path() -> PathBuf {
    crate::config::app_data_dir().join("traffic_stats.json")
}

/// Process-wide stats, loaded once from disk on first access.
fn store() -> &'static Mutex<StatsData> {
    static STORE: OnceLock<Mutex<StatsData>> = OnceLock::new();
    STORE.get_or_init(|| {
        let data = std::fs::read_to_string(stats_path())
            .ok()
            .and_then(|s| serde_json::from_str::<StatsData>(&s).ok())
            .unwrap_or_default();
        Mutex::new(data)
    })
}

fn persist(data: &StatsData) {
    if let Ok(json) = serde_json::to_string(data) {
        let _ = std::fs::write(stats_path(), json);
    }
}

/// Today's date as `YYYY-MM-DD` in the local timezone.
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Record `upload`/`download` bytes against today's bucket and persist.
pub fn record_today(upload: u64, download: u64) {
    if upload == 0 && download == 0 {
        return;
    }
    let mut guard = store().lock().unwrap_or_else(|e| e.into_inner());
    guard.add_sample(&today(), upload, download);
    persist(&guard);
}

/// Recent daily history, oldest-first.
pub fn history(days: usize) -> Vec<TrafficDay> {
    store().lock().unwrap_or_else(|e| e.into_inner()).recent(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sample_accumulates_per_day() {
        let mut s = StatsData::default();
        s.add_sample("2026-06-25", 100, 200);
        s.add_sample("2026-06-25", 50, 25);
        s.add_sample("2026-06-26", 1, 1);
        assert_eq!(s.days["2026-06-25"], DailyTraffic { upload: 150, download: 225 });
        assert_eq!(s.days["2026-06-26"], DailyTraffic { upload: 1, download: 1 });
    }

    #[test]
    fn recent_returns_oldest_first_and_limits() {
        let mut s = StatsData::default();
        for d in 1..=5 {
            s.add_sample(&format!("2026-06-0{d}"), d as u64, 0);
        }
        let last3 = s.recent(3);
        assert_eq!(last3.len(), 3);
        assert_eq!(last3[0].date, "2026-06-03");
        assert_eq!(last3[2].date, "2026-06-05");
        // 0 = all
        assert_eq!(s.recent(0).len(), 5);
    }

    #[test]
    fn prune_keeps_only_max_days_newest() {
        let mut s = StatsData::default();
        for d in 0..(MAX_DAYS + 10) {
            // dates like 2026-001 … sortable lexically for the test
            s.add_sample(&format!("2026-{:04}", d), 1, 0);
        }
        assert_eq!(s.days.len(), MAX_DAYS);
        // The oldest 10 must have been pruned; the newest must remain.
        assert!(s.days.contains_key(&format!("2026-{:04}", MAX_DAYS + 9)));
        assert!(!s.days.contains_key("2026-0000"));
    }

    #[test]
    fn saturating_add_never_panics_on_overflow() {
        let mut s = StatsData::default();
        s.add_sample("d", u64::MAX, u64::MAX);
        s.add_sample("d", 10, 10);
        assert_eq!(s.days["d"], DailyTraffic { upload: u64::MAX, download: u64::MAX });
    }
}

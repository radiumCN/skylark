use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize)]
pub struct TrafficTotals {
    #[serde(rename(deserialize = "uploadTotal"))]
    upload: u64,
    #[serde(rename(deserialize = "downloadTotal"))]
    download: u64,
}

impl TrafficTotals {
    fn delta_since(self, previous: Self) -> Self {
        Self {
            upload: self.upload.checked_sub(previous.upload).unwrap_or(self.upload),
            download: self.download.checked_sub(previous.download).unwrap_or(self.download),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Deserialize, Serialize)]
pub struct TrafficRate {
    up: u64,
    down: u64,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct TrafficSnapshot {
    totals: TrafficTotals,
    rate: TrafficRate,
}

/// One instance per core process. Cumulative samples feed the daily history, and the
/// streaming deltas drive live speed. Keeping those paths independent avoids counting
/// a byte twice around stream reconnects or losing it when the WebView is hidden.
#[derive(Debug)]
pub struct TrafficMonitor {
    api_port: u16,
    totals: Mutex<TrafficTotals>,
    rate: Mutex<Option<(Instant, TrafficRate)>>,
    checkpoint_lock: tokio::sync::Mutex<()>,
    refresh: Notify,
}

impl TrafficMonitor {
    pub fn new(api_port: u16) -> Self {
        Self {
            api_port,
            totals: Mutex::default(),
            rate: Mutex::default(),
            checkpoint_lock: tokio::sync::Mutex::new(()),
            refresh: Notify::new(),
        }
    }

    pub fn snapshot(&self) -> TrafficSnapshot {
        let totals = *self.totals.lock().unwrap_or_else(|e| e.into_inner());
        let rate = *self.rate.lock().unwrap_or_else(|e| e.into_inner());
        TrafficSnapshot {
            totals,
            rate: rate
                .filter(|(time, _)| time.elapsed() < Duration::from_secs(3))
                .map(|(_, rate)| rate)
                .unwrap_or_default(),
        }
    }

    pub fn refresh(&self) {
        self.refresh.notify_one();
    }

    pub async fn finish(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.checkpoint()).await;
    }

    async fn checkpoint(&self) {
        // Serialize scheduled samples and the final sample taken on disconnect. On
        // failure, keep the last good totals so the next checkpoint can recover the delta.
        let _guard = self.checkpoint_lock.lock().await;
        let Ok(totals) = self.fetch_totals().await else { return };
        let delta = {
            let mut previous = self.totals.lock().unwrap_or_else(|e| e.into_inner());
            let delta = totals.delta_since(*previous);
            *previous = totals;
            delta
        };
        crate::stats::record_today(delta.upload, delta.download);
    }

    async fn fetch_totals(&self) -> Result<TrafficTotals> {
        // serde skips the connections array without allocating a tree for its entries.
        Ok(
            crate::singbox::clash_request(reqwest::Method::GET, self.api_port, "/connections")?
                .timeout(Duration::from_secs(2))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?,
        )
    }

    pub async fn run(&self, tun_mode: bool) {
        tokio::join!(self.run_rate_stream(), self.run_checkpoints(tun_mode));
    }

    async fn run_checkpoints(&self, tun_mode: bool) {
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(1),
            CHECKPOINT_INTERVAL,
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = self.refresh.notified() => {
                    interval.reset();
                }
            }
            if tun_mode || tokio::task::spawn_blocking(crate::proxy::get_system_proxy_status)
                .await.unwrap_or(false)
            {
                self.checkpoint().await;
            }
        }
    }

    async fn run_rate_stream(&self) {
        loop {
            let _ = self.read_rates().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn read_rates(&self) -> Result<()> {
        let mut response =
            crate::singbox::clash_request(reqwest::Method::GET, self.api_port, "/traffic")?
                .send()
                .await?
                .error_for_status()?;
        let mut pending = Vec::with_capacity(64);
        while let Some(chunk) = response.chunk().await? {
            self.apply_rate_chunk(&mut pending, &chunk)?;
        }
        Ok(())
    }

    fn apply_rate_chunk(&self, pending: &mut Vec<u8>, chunk: &[u8]) -> Result<()> {
        // HTTP chunks can split a frame or carry several newline-delimited frames.
        for part in chunk.split_inclusive(|&byte| byte == b'\n') {
            ensure!(pending.len() + part.len() <= 512, "oversized sing-box traffic frame");
            pending.extend_from_slice(part);
            if part.ends_with(b"\n") {
                let rate = serde_json::from_slice(pending)?;
                *self.rate.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), rate));
                pending.clear();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_totals_without_connection_fields_and_counts_each_sample_once() {
        let totals: TrafficTotals =
            serde_json::from_str(r#"{"uploadTotal":100,"downloadTotal":200,"connections":[{"id":"1"}]}"#)
                .unwrap();
        assert_eq!(totals.delta_since(TrafficTotals::default()).upload, 100);
        assert_eq!(totals.delta_since(totals).download, 0);
        let after_reset = TrafficTotals { upload: 10, download: 250 }.delta_since(totals);
        assert_eq!(after_reset.upload, 10);
        assert_eq!(after_reset.download, 50);
        assert!(serde_json::from_str::<TrafficTotals>(r#"{"message":"temporarily unavailable"}"#).is_err());
    }

    #[test]
    fn reads_partial_and_coalesced_rate_frames_with_bounded_buffer() {
        let monitor = TrafficMonitor::new(0);
        let mut pending = Vec::new();
        monitor.apply_rate_chunk(&mut pending, b"{\"up\":15,").unwrap();
        assert_eq!(monitor.snapshot().rate.up, 0);

        monitor
            .apply_rate_chunk(&mut pending, b"\"down\":25}\n{\"up\":35,\"down\":45}\n")
            .unwrap();
        let latest = monitor.snapshot().rate;
        assert_eq!((latest.up, latest.down), (35, 45));
        assert!(pending.is_empty());
        assert!(monitor.apply_rate_chunk(&mut pending, &[b' '; 513]).is_err());
    }
}

use std::sync::Arc;
use std::time::Duration;

use eoka::Page;

use crate::browser::manager::BrowserManager;

#[derive(Debug, Clone)]
pub struct TabSnapshot {
    pub id: String,
    pub url: String,
    #[allow(dead_code)]
    pub title: String,
}

pub struct TargetFilter {
    pub url_contains: Option<String>,
}

impl TargetFilter {
    fn matches(&self, tab: &TabSnapshot) -> bool {
        match &self.url_contains {
            Some(pattern) => tab.url.to_lowercase().contains(&pattern.to_lowercase()),
            None => true,
        }
    }
}

/// General-purpose tab lifecycle monitor.
///
/// Replaces `PopupWatcher` with a broader capability set:
/// - Detect new tabs (with optional URL filtering)
/// - Detect tab closures
/// - Attach to any detected tab
/// - Configurable polling interval (default 200ms)
///
/// Architecture: polling-based via `Target.getTargets`. The interface is
/// designed so a future CDP event-driven backend (`Target.setDiscoverTargets`)
/// can be swapped in without changing callers.
pub struct TargetWatcher {
    bm: Arc<BrowserManager>,
    baseline: Vec<TabSnapshot>,
    filter: Option<TargetFilter>,
    poll_interval: Duration,
}

impl TargetWatcher {
    /// Create a watcher with a baseline snapshot of current tabs.
    pub async fn new(bm: Arc<BrowserManager>) -> anyhow::Result<Self> {
        let baseline = Self::snapshot(&bm).await?;
        Ok(Self {
            bm,
            baseline,
            filter: None,
            poll_interval: Duration::from_millis(200),
        })
    }

    /// Create with a URL filter — only tabs matching the pattern are reported.
    pub async fn with_filter(
        bm: Arc<BrowserManager>,
        url_contains: Option<String>,
    ) -> anyhow::Result<Self> {
        let baseline = Self::snapshot(&bm).await?;
        let filter = url_contains.map(|uc| TargetFilter {
            url_contains: Some(uc),
        });
        Ok(Self {
            bm,
            baseline,
            filter,
            poll_interval: Duration::from_millis(200),
        })
    }

    /// Wait for a new tab that wasn't in the baseline.
    /// Returns the first matching new tab, or None on timeout.
    pub async fn wait_for_new(&self, timeout_secs: u64) -> Option<TabSnapshot> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        let mut consecutive_errors = 0u32;

        while tokio::time::Instant::now() < deadline {
            match Self::snapshot(&self.bm).await {
                Ok(current) => {
                    consecutive_errors = 0;
                    let baseline_ids: Vec<&str> =
                        self.baseline.iter().map(|t| t.id.as_str()).collect();

                    let new_tabs: Vec<&TabSnapshot> = current
                        .iter()
                        .filter(|t| !baseline_ids.contains(&t.id.as_str()))
                        .collect();

                    if new_tabs.is_empty() {
                        // nothing new
                    } else if let Some(ref filter) = self.filter {
                        // With filter: prefer matching tabs
                        if let Some(tab) = new_tabs.iter().find(|t| filter.matches(t)) {
                            tracing::debug!(
                                target_id = %tab.id, url = %tab.url,
                                "New target detected (filtered)"
                            );
                            return Some((*tab).clone());
                        }
                    } else {
                        if let Some(tab) = new_tabs.iter().find(|t| !t.url.is_empty() && t.url != "about:blank") {
                            tracing::debug!(
                                target_id = %tab.id, url = %tab.url,
                                "New target detected"
                            );
                            return Some((*tab).clone());
                        }
                        if let Some(tab) = new_tabs.first() {
                            tracing::debug!(
                                target_id = %tab.id, url = %tab.url,
                                "New target detected (about:blank, will settle later)"
                            );
                            return Some((*tab).clone());
                        }
                    }
                }
                Err(e) => {
                    consecutive_errors += 1;
                    tracing::warn!(
                        error = %e, consecutive_errors,
                        "Failed to list tabs in TargetWatcher"
                    );
                    if consecutive_errors >= 5 {
                        tracing::error!("Too many consecutive failures, aborting watch");
                        return None;
                    }
                }
            }
            tokio::time::sleep(self.poll_interval).await;
        }
        tracing::debug!("No new target within {}s", timeout_secs);
        None
    }

    /// Wait for a specific tab to close (disappear from the tab list).
    pub async fn wait_for_close(&self, target_id: &str, timeout_secs: u64) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

        while tokio::time::Instant::now() < deadline {
            if let Ok(tabs) = Self::snapshot(&self.bm).await {
                if !tabs.iter().any(|t| t.id == target_id) {
                    tracing::info!(target_id, "Target closed");
                    return true;
                }
            }
            tokio::time::sleep(self.poll_interval).await;
        }
        tracing::debug!(target_id, "Target did not close within {}s", timeout_secs);
        false
    }

    /// Attach to a target tab and return its Page handle.
    pub async fn attach(&self, target_id: &str) -> anyhow::Result<Page> {
        self.bm.attach_tab(target_id).await
    }

    async fn snapshot(bm: &BrowserManager) -> anyhow::Result<Vec<TabSnapshot>> {
        let tabs = bm.list_tabs().await?;
        Ok(tabs
            .into_iter()
            .map(|t| TabSnapshot {
                id: t.id,
                url: t.url,
                title: t.title,
            })
            .collect())
    }
}

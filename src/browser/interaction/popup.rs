use std::sync::Arc;
use eoka::Page;
use crate::browser::manager::BrowserManager;
use super::signals::AUTH_URL_PATTERNS;

fn is_likely_auth_popup(url: &str) -> bool {
    let u = url.to_lowercase();
    AUTH_URL_PATTERNS.iter().any(|p| u.contains(&p.to_lowercase()))
}

pub struct PopupWatcher {
    bm: Arc<BrowserManager>,
    baseline_ids: Vec<String>,
}

impl PopupWatcher {
    pub async fn new(bm: Arc<BrowserManager>) -> anyhow::Result<Self> {
        let tabs = bm.list_tabs().await?;
        let baseline_ids: Vec<String> = tabs.into_iter().map(|t| t.id).collect();
        Ok(Self { bm, baseline_ids })
    }

    /// Poll for a new tab that wasn't present at snapshot time.
    /// Returns the target_id of the new tab, or None if timeout.
    pub async fn wait_for_popup(&self, timeout_secs: u64) -> Option<String> {
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(timeout_secs);
        let mut consecutive_errors = 0u32;

        while tokio::time::Instant::now() < deadline {
            match self.bm.list_tabs().await {
                Ok(tabs) => {
                    consecutive_errors = 0;
                    for tab in &tabs {
                        if !self.baseline_ids.contains(&tab.id)
                            && is_likely_auth_popup(&tab.url)
                        {
                            tracing::info!(target_id = %tab.id, url = %tab.url, "Auth popup detected");
                            return Some(tab.id.clone());
                        }
                    }
                    // Fallback: any new page-type tab
                    for tab in &tabs {
                        if !self.baseline_ids.contains(&tab.id) {
                            tracing::info!(target_id = %tab.id, url = %tab.url, "New tab detected (non-auth URL)");
                            return Some(tab.id.clone());
                        }
                    }
                }
                Err(e) => {
                    consecutive_errors += 1;
                    tracing::warn!(error = %e, consecutive_errors, "Failed to list tabs while watching for popup");
                    if consecutive_errors >= 5 {
                        tracing::error!("Too many consecutive tab listing failures, aborting popup watch");
                        return None;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        tracing::debug!("No popup detected within {}s", timeout_secs);
        None
    }

    /// Attach to a popup tab and return its Page handle.
    pub async fn attach(&self, target_id: &str) -> anyhow::Result<Page> {
        self.bm.attach_tab(target_id).await
    }

    /// Wait for a popup tab to close (disappear from the tab list).
    pub async fn wait_for_close(&self, target_id: &str, timeout_secs: u64) -> bool {
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(timeout_secs);

        while tokio::time::Instant::now() < deadline {
            if let Ok(tabs) = self.bm.list_tabs().await {
                if !tabs.iter().any(|t| t.id == target_id) {
                    tracing::info!(target_id, "Popup closed");
                    return true;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        tracing::debug!(target_id, "Popup did not close within {}s", timeout_secs);
        false
    }
}

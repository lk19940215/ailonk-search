use std::sync::Arc;
use eoka::{Browser, Page};
use tokio::sync::Semaphore;

pub struct TabPool {
    browser: Arc<Browser>,
    semaphore: Arc<Semaphore>,
    max_tabs: usize,
}

impl TabPool {
    pub fn new(browser: Arc<Browser>, max_tabs: usize) -> Self {
        Self {
            browser,
            semaphore: Arc::new(Semaphore::new(max_tabs)),
            max_tabs,
        }
    }

    pub async fn acquire(&self) -> anyhow::Result<TabGuard> {
        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.semaphore.clone().acquire_owned(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "Tab pool exhausted: all {} tabs in use, timed out after 5s",
                self.max_tabs
            )
        })?
        .map_err(|e| anyhow::anyhow!("Semaphore closed: {e}"))?;

        let page = self.browser.new_blank_page().await
            .map_err(|e| anyhow::anyhow!("Failed to create new tab: {}", e))?;

        let target_id = page.target_id().to_string();

        Ok(TabGuard {
            page: Some(page),
            browser: self.browser.clone(),
            target_id,
            closed: false,
            _permit: permit,
        })
    }

    pub fn max_tabs(&self) -> usize {
        self.max_tabs
    }
}

pub struct TabGuard {
    page: Option<Page>,
    browser: Arc<Browser>,
    target_id: String,
    closed: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl TabGuard {
    pub fn page(&self) -> &Page {
        self.page.as_ref().expect("TabGuard page already taken")
    }

    pub async fn close(mut self) {
        self.closed = true;
        self.page.take();
        self.browser.close_tab(&self.target_id).await.ok();
    }
}

impl Drop for TabGuard {
    fn drop(&mut self) {
        if !self.closed {
            tracing::warn!("TabGuard dropped without close() — spawning cleanup task");
            self.page.take();
            let browser = self.browser.clone();
            let target_id = self.target_id.clone();
            tokio::spawn(async move {
                if let Err(e) = browser.close_tab(&target_id).await {
                    tracing::warn!(error = %e, "Failed to close orphan tab");
                }
            });
        }
    }
}

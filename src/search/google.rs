use async_trait::async_trait;
use eoka::Page;

use crate::browser::interaction;
use super::engine::{SearchEngine, SearchResult};

pub struct GoogleEngine {
    pub nav_timeout: u64,
}

const GOOGLE_RESULTS_JS: &str = r#"
    (() => {
        let items = document.querySelectorAll('div.g:not([data-text-ad])');
        let results = Array.from(items)
            .filter(el => el.querySelector('h3') && el.querySelector('a[href]'))
            .map(el => ({
                title: el.querySelector('h3')?.textContent?.trim() || '',
                url: el.querySelector('a[href]')?.href || '',
                snippet: (el.querySelector('[data-sncf]') || el.querySelector('.VwiC3b') || el.querySelector('.lEBKkf') || el.querySelector('span.st'))?.textContent?.trim() || ''
            }))
            .filter(r => r.title && r.url && !r.url.startsWith('https://www.google.com'));

        if (results.length > 0) return results;

        const searchArea = document.querySelector('#search, #rso, #main');
        if (!searchArea) return [];

        const h3s = searchArea.querySelectorAll('h3');
        results = Array.from(h3s).map(h3 => {
            const link = h3.closest('a') || h3.parentElement?.querySelector('a');
            if (!link || !link.href) return null;
            const container = h3.closest('[data-hveid], [data-sokoban-container], [data-ved]') || h3.parentElement?.parentElement;
            const snippetEl = container?.querySelector('[data-sncf], .VwiC3b, .lEBKkf, span[style*="-webkit-line-clamp"]');
            return {
                title: h3.textContent?.trim() || '',
                url: link.href,
                snippet: snippetEl?.textContent?.trim() || ''
            };
        })
        .filter(r => r && r.title && r.url
            && !r.url.startsWith('https://www.google.com')
            && !r.url.includes('google.com/search'));

        return results;
    })()
"#;

#[async_trait]
impl SearchEngine for GoogleEngine {
    fn name(&self) -> &str {
        "google"
    }

    async fn search(
        &self,
        page: &Page,
        query: &str,
        count: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "https://www.google.com/search?q={}&num={}&hl=en",
            urlencoding::encode(query),
            count.min(20)
        );

        interaction::navigate(page, &url, self.nav_timeout).await?;
        interaction::handle_consent(page, "google").await?;
        page.wait_for_any(&["div.g", "#search", "#rso"], 10000).await.ok();

        match interaction::resolve_captcha_loop(page, 2).await {
            Ok(true) => {
                page.wait(1500).await;
                page.wait_for_any(&["div.g", "#search", "#rso"], 10000).await.ok();
            }
            Err(e) => return Err(e),
            Ok(false) => {}
        }

        let mut results: Vec<SearchResult> = interaction::extract(page, GOOGLE_RESULTS_JS).await?;

        if results.is_empty() {
            if interaction::is_captcha_present(page).await {
                match interaction::resolve_captcha_loop(page, 1).await {
                    Ok(true) => {
                        page.wait(1500).await;
                        page.wait_for_any(&["div.g", "#search", "#rso"], 10000).await.ok();
                        results = interaction::extract(page, GOOGLE_RESULTS_JS).await?;
                    }
                    Err(e) => return Err(e),
                    Ok(false) => {}
                }
            }
            if results.is_empty() {
                let title = page.title().await.unwrap_or_default();
                tracing::warn!(page_title = %title, "Google returned 0 results");
            }
        }

        Ok(results.into_iter().take(count).collect())
    }
}

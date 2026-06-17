use async_trait::async_trait;
use eoka::Page;

use crate::browser::interaction;
use super::engine::{SearchEngine, SearchResult};

pub struct DuckDuckGoEngine;

const DDG_RESULTS_JS: &str = r#"
    (() => {
        const items = document.querySelectorAll('.result, .web-result');
        return Array.from(items).map(el => {
            const a = el.querySelector('.result__a, a.result__url, a');
            const snippetEl = el.querySelector('.result__snippet, .result__body');
            const title = a?.textContent?.trim() || '';
            let url = a?.href || '';
            try {
                const parsed = new URL(url);
                const uddg = parsed.searchParams.get('uddg');
                if (uddg) url = decodeURIComponent(uddg);
            } catch {}
            const snippet = snippetEl?.textContent?.trim() || '';
            return { title, url, snippet };
        }).filter(r => r.title && r.url && !r.url.includes('duckduckgo.com'));
    })()
"#;

#[async_trait]
impl SearchEngine for DuckDuckGoEngine {
    fn name(&self) -> &str {
        "duckduckgo"
    }

    async fn search(
        &self,
        page: &Page,
        query: &str,
        count: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(query),
        );

        interaction::navigate(page, &url, 15).await?;
        page.wait_for_any(&[".result", ".web-result"], 10000).await.ok();
        page.wait(500).await;

        let title = page.title().await.unwrap_or_default();
        if title.is_empty() || title.to_lowercase().contains("blocked") {
            anyhow::bail!("DuckDuckGo appears to have blocked the request");
        }

        let results: Vec<SearchResult> = interaction::extract(page, DDG_RESULTS_JS).await?;
        Ok(results.into_iter().take(count).collect())
    }
}

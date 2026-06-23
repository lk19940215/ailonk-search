use async_trait::async_trait;
use eoka::Page;

use crate::browser::interaction;
use super::engine::{SearchEngine, SearchResult};

pub struct BingEngine {
    pub region: &'static str,
    pub nav_timeout: u64,
}

fn is_garbage_result(r: &SearchResult) -> bool {
    const GARBAGE_DOMAINS: &[&str] = &[
        "miit.gov.cn",
        "mps.gov.cn",
        "beian.miit.gov.cn",
        "bing.com/ck/a?!",
        "go.microsoft.com",
    ];
    const GARBAGE_TITLES: &[&str] = &[
        "增值电信业务经营许可证",
        "京ICP备",
        "京公网安备",
        "ICP证",
    ];
    let url_lower = r.url.to_lowercase();
    let title_lower = r.title.to_lowercase();
    GARBAGE_DOMAINS.iter().any(|d| url_lower.contains(d))
        || GARBAGE_TITLES.iter().any(|t| title_lower.contains(&t.to_lowercase()))
}

fn clean_bing_url(raw: &str) -> String {
    if !raw.contains("bing.com/ck/a") {
        return raw.to_string();
    }
    if let Ok(parsed) = url::Url::parse(raw) {
        for (key, val) in parsed.query_pairs() {
            if key == "u" {
                if let Some(encoded) = val.strip_prefix("a1") {
                    if let Ok(decoded) = urlencoding::decode(encoded) {
                        return decoded.into_owned();
                    }
                }
            }
        }
    }
    raw.to_string()
}

const BING_RESULTS_JS: &str = r#"
    (() => {
        const items = document.querySelectorAll('li.b_algo, .b_algo');
        if (items.length > 0) {
            return Array.from(items).map(el => ({
                title: (el.querySelector('h2 a') || el.querySelector('h2'))?.textContent?.trim() || '',
                url: (el.querySelector('h2 a') || el.querySelector('a'))?.href || '',
                snippet: (el.querySelector('.b_caption p') || el.querySelector('p') || el.querySelector('.b_lineclamp2'))?.textContent?.trim() || ''
            })).filter(r => r.title && r.url);
        }

        const degradedSelectors = ['#b_results > li', '.b_results > li', 'ol#b_results > li'];
        for (const sel of degradedSelectors) {
            const nodes = document.querySelectorAll(sel);
            if (nodes.length > 0) {
                const results = [];
                nodes.forEach(el => {
                    const a = el.querySelector('h2 a') || el.querySelector('h3 a') || el.querySelector('a[href^="http"]');
                    const title = (el.querySelector('h2') || el.querySelector('h3') || a)?.textContent?.trim();
                    const snippet = (el.querySelector('p') || el.querySelector('.b_lineclamp2'))?.textContent?.trim() || '';
                    if (a && title && a.href && !a.href.includes('bing.com/aclick')) {
                        results.push({ title, url: a.href, snippet });
                    }
                });
                if (results.length > 0) return results;
            }
        }
        return [];
    })()
"#;

#[async_trait]
impl SearchEngine for BingEngine {
    fn name(&self) -> &str {
        "bing"
    }

    async fn search(
        &self,
        page: &Page,
        query: &str,
        count: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let domain = if self.region == "cn" { "cn.bing.com" } else { "www.bing.com" };

        // Navigate to Bing homepage and use human-like typing
        let homepage = format!("https://{}/", domain);
        interaction::navigate(page, &homepage, self.nav_timeout).await?;
        interaction::handle_consent(page, "bing").await?;

        let search_selectors = &[
            "#sb_form_q",
            "input[name=\"q\"]",
            "textarea[name=\"q\"]",
            ".b_searchbox",
        ];

        let typed = interaction::type_and_submit(page, search_selectors, query, 8000).await?;

        if !typed {
            // Fallback: direct URL navigation
            let direct_url = format!(
                "https://{}/search?q={}&count={}",
                domain, urlencoding::encode(query), count.min(20)
            );
            interaction::navigate(page, &direct_url, self.nav_timeout).await?;
        }

        interaction::handle_consent(page, "bing").await?;

        page.wait_for_any(&["li.b_algo", ".b_algo", "#b_results"], 15000).await.ok();

        // CAPTCHA check
        match interaction::resolve_captcha_loop(page, 2).await {
            Ok(true) => {
                page.wait(1500).await;
                page.wait_for_any(&["li.b_algo", ".b_algo", "#b_results"], 10000).await.ok();
            }
            Err(e) => return Err(e),
            Ok(false) => {}
        }

        let mut results: Vec<SearchResult> = interaction::extract(page, BING_RESULTS_JS).await?;

        if results.is_empty() {
            if interaction::is_captcha_present(page).await {
                match interaction::resolve_captcha_loop(page, 1).await {
                    Ok(true) => {
                        page.wait(1000).await;
                        results = interaction::extract(page, BING_RESULTS_JS).await?;
                    }
                    Err(e) => return Err(e),
                    Ok(false) => {}
                }
            }
        }

        if results.is_empty() {
            let title = page.title().await.unwrap_or_default();
            let url = page.url().await.unwrap_or_default();
            tracing::warn!(page_title = %title, page_url = %url, "Bing returned 0 results");
        }

        let results: Vec<SearchResult> = results
            .into_iter()
            .map(|mut r| { r.url = clean_bing_url(&r.url); r })
            .filter(|r| !is_garbage_result(r))
            .take(count)
            .collect();

        Ok(results)
    }
}

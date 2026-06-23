use async_trait::async_trait;
use eoka::Page;

use crate::browser::interaction;
use super::engine::{SearchEngine, SearchResult};

pub struct GoogleEngine {
    pub region: &'static str,
    pub nav_timeout: u64,
}

const GOOGLE_RESULTS_JS: &str = r#"
    (() => {
        const SNIP_SEL = '[data-sncf], .VwiC3b, .lEBKkf, span.st, span[style*="-webkit-line-clamp"]';
        const CONT_SEL = '.MjjYud, div.g, [data-sokoban-container]';
        const SKIP_SEL = 'a, script, style, cite, [aria-hidden="true"], .VuuXrf, .XNo5Ab, .qLRx3b, [data-text-ad], .ULSxyf, .ylgVCe, .ojE3Fb, .B6fmyf, .byrV5b';
        const isBad = (t, title) => {
            t = (t || '').replace(/\s+/g, ' ').trim();
            if (t.length <= 20 || t === title) return true;
            if (/^(广告|Sponsored|Ad|赞助|推广)$/i.test(t)) return true;
            if (/翻译此页|Translate this page/i.test(t)) return true;
            if (/^[\w.-]+\.(com|org|net|io|edu|gov|cn)([\s›>]|$)/i.test(t) && t.length < 80) return true;
            if (/https?:\/\//.test(t) && t.length < 100) return true;
            return false;
        };
        const fromEl = (el, title, h3) => {
            if (!el || el.closest('a')) return '';
            const c = el.cloneNode(true);
            c.querySelectorAll(SKIP_SEL).forEach(n => n.remove());
            if (h3) c.querySelectorAll('h3').forEach(n => n.remove());
            const t = c.textContent?.replace(/\s+/g, ' ').trim() || '';
            return isBad(t, title) ? '' : t;
        };
        const byClass = (root) => {
            const t = root.querySelector(SNIP_SEL)?.textContent?.replace(/\s+/g, ' ').trim() || '';
            return t.length > 20 && !isBad(t, '') ? t : '';
        };
        const byStructure = (root, h3, title) => {
            if (!root || !h3) return '';
            const link = h3.closest('a');
            const blocks = [];
            if (link) blocks.push(link, link.parentElement);
            const urlBlock = h3.closest('.yuRUbf');
            if (urlBlock) blocks.push(urlBlock);
            blocks.push(h3.closest('div'), h3.parentElement, h3.parentElement?.parentElement);
            const seen = new Set();
            for (const start of blocks) {
                if (!start || seen.has(start)) continue;
                seen.add(start);
                for (let s = start.nextElementSibling; s; s = s.nextElementSibling) {
                    const t = fromEl(s, title, h3);
                    if (t) return t;
                }
            }
            for (const el of root.querySelectorAll('div, span, p')) {
                if (el === h3 || h3.contains(el)) continue;
                if (el.closest('a') || el.matches(SKIP_SEL)) continue;
                if (!(h3.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING)) continue;
                const t = fromEl(el, title, h3);
                if (t) return t;
            }
            return '';
        };
        const getSnippet = (root, h3, title) => byClass(root) || byStructure(root, h3, title);
        const getContainer = (h3) => h3.closest(CONT_SEL) || h3.parentElement?.parentElement?.parentElement;
        const buildResult = (h3, url) => {
            const title = h3?.textContent?.trim() || '';
            if (!title || !url) return null;
            const container = getContainer(h3);
            return { title, url, snippet: getSnippet(container || document.body, h3, title) };
        };
        const isValidUrl = (url) => url && !url.startsWith('https://www.google.com') && !url.includes('google.com/search');

        let results = Array.from(document.querySelectorAll('a:has(h3)'))
            .map(a => buildResult(a.querySelector('h3'), a.href))
            .filter(r => r && isValidUrl(r.url));

        if (results.length > 0) return results;

        results = Array.from(document.querySelectorAll('div.g:not([data-text-ad])'))
            .filter(el => el.querySelector('h3') && el.querySelector('a[href]'))
            .map(el => buildResult(el.querySelector('h3'), el.querySelector('a[href]')?.href))
            .filter(r => r && isValidUrl(r.url));

        if (results.length > 0) return results;

        const searchArea = document.querySelector('#search, #rso, #main');
        if (!searchArea) return [];

        results = Array.from(searchArea.querySelectorAll('h3'))
            .map(h3 => {
                const link = h3.closest('a') || h3.parentElement?.querySelector('a');
                return link?.href ? buildResult(h3, link.href) : null;
            })
            .filter(r => r && isValidUrl(r.url));

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
        let hl = if self.region == "cn" { "zh-CN" } else { "en" };
        let url = format!(
            "https://www.google.com/search?q={}&num={}&hl={}",
            urlencoding::encode(query),
            count.min(20),
            hl,
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

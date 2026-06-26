pub mod tools;
mod authorize;
mod popup_handler;

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
};

use crate::browser::interaction;
use crate::browser::manager::{BrowserManager, LazyBrowserManager};
use crate::cache::{CachedContent, ContentCache};
use crate::search::engine::{
    format_search_results, search_with_fallback, select_engine, to_mcp_error, RateLimiter,
};

#[derive(Clone)]
pub struct SearchServer {
    browser: Arc<LazyBrowserManager>,
    pub default_engine: String,
    pub region: &'static str,
    rate_limiter: tokio::sync::OnceCell<Arc<RateLimiter>>,
    pub cache: ContentCache,
    pub allow_private_urls: bool,
}

impl SearchServer {
    pub fn new(
        browser: Arc<LazyBrowserManager>,
        default_engine: String,
        cache_ttl: u64,
        region: &'static str,
        allow_private_urls: bool,
    ) -> Self {
        Self {
            browser,
            default_engine,
            region,
            rate_limiter: tokio::sync::OnceCell::new(),
            cache: ContentCache::new(cache_ttl),
            allow_private_urls,
        }
    }

    /// Get a browser connection. Returns cached healthy connection or triggers reconnect.
    /// No extra spawn/timeout — relies on `with_hard_timeout` at the tool level and
    /// `spawn_connect` at the individual eoka connection level.
    async fn get_browser(&self) -> Result<Arc<BrowserManager>, ErrorData> {
        self.browser.get().await.map_err(|e| to_mcp_error(e.to_string()))
    }

    /// Kill the debug Chrome process on port 19222 so cookie files can be safely overwritten.
    async fn kill_debug_chrome(&self) {
        crate::browser::manager::kill_process_on_port(19222).await;
    }

    fn check_cdp_error<T>(&self, result: &Result<T, ErrorData>, bm: &BrowserManager) {
        if let Err(e) = result {
            if is_fatal_cdp_error(&e.message) {
                bm.mark_unhealthy();
            }
        }
    }
}

fn is_fatal_cdp_error(msg: &str) -> bool {
    msg.contains("connection is dead")
        || msg.contains("Transport error")
        || msg.contains("CDP reader closed")
        || msg.contains("CDP reader thread")
        || msg.contains("WebSocket connection closed")
        || msg.contains("broken pipe")
        || msg.contains("Connection reset")
}

pub(crate) fn is_fatal_cdp_error_anyhow(err: &anyhow::Error) -> bool {
    is_fatal_cdp_error(&err.to_string())
}

/// Hard ceiling for fetch_and_extract: prevents CDP hangs from holding tabs forever.
const FETCH_HARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

/// Tab acquisition timeout — catches the case where Chrome cannot create/reuse tabs
/// (e.g. popup is blocking page operations or Chrome is partially unresponsive).
const TAB_ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Spawn-isolated hard timeout: runs `work` on a separate tokio task and enforces
/// the timeout via a `oneshot` channel. This guarantees the timeout fires even if
/// the work blocks a tokio worker thread (e.g. blocking CDP I/O), because the
/// `tokio::time::timeout` waits on a pure in-memory channel, not on the work itself.
async fn with_hard_timeout<F>(
    timeout: std::time::Duration,
    name: &'static str,
    work: F,
) -> Result<CallToolResult, ErrorData>
where
    F: std::future::Future<Output = Result<CallToolResult, ErrorData>> + Send + 'static,
{
    let start = std::time::Instant::now();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _ = tx.send(work.await);
    });
    let result = match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(to_mcp_error(format!("{}: internal task cancelled", name))),
        Err(_) => {
            tracing::error!("{} hard timeout ({}s) — CDP may be blocked", name, timeout.as_secs());
            Err(to_mcp_error(format!(
                "{}: hard timeout after {}s. Browser may be unresponsive — please restart MCP server.",
                name, timeout.as_secs()
            )))
        }
    };
    let elapsed = start.elapsed();
    if result.is_err() {
        tracing::warn!("{}: returning error after {:.1}s", name, elapsed.as_secs_f64());
    } else {
        tracing::debug!("{}: completed in {:.1}s", name, elapsed.as_secs_f64());
    }
    result
}

/// Acquire a tab from the pool with the standard timeout.
async fn acquire_tab(bm: &BrowserManager) -> Result<crate::browser::pool::TabGuard, ErrorData> {
    tokio::time::timeout(TAB_ACQUIRE_TIMEOUT, bm.tab_pool().acquire())
        .await
        .map_err(|_| to_mcp_error(format!("Tab acquire timed out ({}s)", TAB_ACQUIRE_TIMEOUT.as_secs())))?
        .map_err(to_mcp_error)
}

/// Acquire a tab (returns anyhow::Error for non-MCP contexts like fetch_and_extract).
async fn acquire_tab_anyhow(bm: &BrowserManager) -> anyhow::Result<crate::browser::pool::TabGuard> {
    tokio::time::timeout(TAB_ACQUIRE_TIMEOUT, bm.tab_pool().acquire())
        .await
        .map_err(|_| anyhow::anyhow!("Tab acquire timed out ({}s) — Chrome may be unresponsive", TAB_ACQUIRE_TIMEOUT.as_secs()))?
}

/// Navigate to URL, handle consent/CAPTCHA, and return raw HTML content.
/// Shared by `read_page` and `fetch_and_extract` to eliminate duplication.
async fn navigate_and_fetch_html(
    page: &eoka::Page,
    url: &str,
) -> anyhow::Result<String> {
    interaction::navigate(page, url, 15).await?;
    interaction::handle_consent(page, "").await.ok();
    if interaction::is_captcha_present(page).await {
        tracing::warn!(url = %url, "CAPTCHA detected");
        match interaction::resolve_captcha_loop(page, 1).await {
            Ok(_) => { let _ = page.wait_for_network_idle(500, 3000).await; }
            Err(_) => anyhow::bail!("[READ_FAILED] CAPTCHA unresolved"),
        }
    }
    page.content().await
        .map_err(|e| anyhow::anyhow!("Content fetch failed: {}", e))
}

/// Shared read logic: acquire tab → navigate → CAPTCHA → extract content.
/// Used by batch_read and search_and_read to eliminate code duplication.
async fn fetch_and_extract(
    bm: &BrowserManager,
    url: &str,
    max_len: usize,
    cache: &ContentCache,
) -> anyhow::Result<crate::extract::content::ExtractedContent> {
    let tab = acquire_tab_anyhow(bm).await?;
    let page_result = tokio::time::timeout(
        FETCH_HARD_TIMEOUT,
        navigate_and_fetch_html(tab.page(), url),
    ).await;
    tab.close().await;

    let html = match page_result {
        Ok(Ok(html)) => html,
        Ok(Err(e)) => {
            if is_fatal_cdp_error_anyhow(&e) { bm.mark_unhealthy(); }
            return Err(e);
        }
        Err(_) => {
            tracing::error!(url = %url, "fetch_and_extract hard timeout — CDP may be unresponsive");
            bm.mark_unhealthy();
            anyhow::bail!("Page fetch timed out. Browser connection may be lost.")
        }
    };

    if html.is_empty() {
        anyhow::bail!("Empty content");
    }
    let extracted = crate::extract::content::ContentExtractor::extract(&html, url, max_len)?;
    if extracted.is_low_quality() {
        let reason = extracted.low_quality_reason().unwrap_or("unknown");
        anyhow::bail!("[READ_FAILED] {} (quality: {:.2})", reason, extracted.quality);
    }
    let cache_key = ContentCache::key(url, true, max_len);
    cache.insert(
        cache_key,
        CachedContent { title: extracted.title.clone(), content: extracted.content.clone() },
    ).await;
    Ok(extracted)
}

/// Read multiple URLs concurrently with semaphore control and per-page/global timeouts.
/// Returns (results, timed_out). Each result is (url, Ok(ExtractedContent) | Err).
/// Shared by `batch_read` and `search_and_read` to eliminate duplication.
async fn read_urls_concurrent(
    bm: Arc<BrowserManager>,
    urls: &[String],
    max_len: usize,
    concurrency: usize,
    cache: ContentCache,
) -> (Vec<(String, Result<crate::extract::content::ExtractedContent, anyhow::Error>)>, bool) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = vec![];

    for url in urls {
        let sem = semaphore.clone();
        let url = url.clone();
        let bm = bm.clone();
        let cache = cache.clone();

        handles.push(tokio::spawn(async move {
            let _permit = match sem.acquire().await {
                Ok(p) => p,
                Err(e) => return (url, Err(anyhow::anyhow!("Semaphore error: {e}"))),
            };

            let cache_key = ContentCache::key(&url, true, max_len);
            if let Some(cached) = cache.get(&cache_key).await {
                let fake = crate::extract::content::ExtractedContent {
                    title: cached.title,
                    content: cached.content,
                    quality: 1.0,
                };
                return (url, Ok(fake));
            }

            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                fetch_and_extract(&bm, &url, max_len, &cache),
            )
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Page read timeout after 30s")));

            (url, result)
        }));
    }

    match tokio::time::timeout(
        std::time::Duration::from_secs(60),
        futures::future::join_all(&mut handles),
    ).await {
        Ok(results) => {
            let collected = results.into_iter().map(|r| match r {
                Ok(pair) => pair,
                Err(e) => ("unknown".to_string(), Err(anyhow::anyhow!("Task panic: {e}"))),
            }).collect();
            (collected, false)
        }
        Err(_) => {
            for h in handles { h.abort(); }
            (vec![], true)
        }
    }
}

impl SearchServer {

    async fn rate_limiter(&self, bm: &BrowserManager) -> &Arc<RateLimiter> {
        use crate::browser::manager::ConnectionMode;
        self.rate_limiter
            .get_or_init(|| async {
                let ms = match bm.mode() {
                    ConnectionMode::Headless => 5000,
                    ConnectionMode::UserChrome => 2000,
                };
                Arc::new(RateLimiter::new(ms))
            })
            .await
    }
}

#[tool_router]
impl SearchServer {
    #[tool(description = "Search the web using a real Chrome browser with anti-bot protection. Returns a list of results with titles, URLs, and snippets. Use this when you need search results but don't need to read the full page content. Supports Google, Bing, and DuckDuckGo.")]
    async fn web_search(
        &self,
        Parameters(params): Parameters<tools::WebSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = params.query.trim().to_string();
        if query.is_empty() || query.chars().count() > 500 {
            return Err(to_mcp_error("Query must be 1-500 characters"));
        }
        let this = self.clone();
        with_hard_timeout(std::time::Duration::from_secs(30), "web_search", async move {
            let count = params.count.clamp(1, 20);
            let bm = this.get_browser().await?;
            let rl = this.rate_limiter(&bm).await;
            rl.wait().await;

            let engine = select_engine(
                &params.engine,
                bm.mode(),
                &this.default_engine,
                this.region,
            );

            let tab = acquire_tab(&bm).await?;

            let result = async {
                search_with_fallback(
                    engine,
                    bm.mode(),
                    tab.page(),
                    &query,
                    count,
                    this.region,
                )
                .await
                .map_err(to_mcp_error)
            }
            .await;

            tab.close().await;
            this.check_cdp_error(&result, &bm);

            match &result {
                Ok(_) => rl.reset_penalty().await,
                Err(_) => rl.backoff().await,
            }

            let (engine_name, results) = result?;
            let text = format_search_results(&query, &engine_name, &results);
            Ok(CallToolResult::success(vec![Content::text(text)]))
        }).await
    }

    #[tool(description = "Fetch a single URL using Chrome and extract the main content as clean Markdown. Handles JavaScript-rendered pages and cookie consent. Use this when you have a specific URL and need its content.")]
    async fn read_page(
        &self,
        Parameters(params): Parameters<tools::ReadPageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        interaction::validate_url(&params.url, self.allow_private_urls).map_err(to_mcp_error)?;
        let max_length = params.max_length.clamp(1, 15000);
        let cache_key = ContentCache::key(&params.url, params.include_links, max_length);
        if let Some(cached) = self.cache.get(&cache_key).await {
            tracing::debug!(url = %params.url, "Cache hit");
            let text = format!(
                "# {}\n\nSource: {} (cached)\n\n---\n\n{}",
                cached.title, params.url, cached.content
            );
            return Ok(CallToolResult::success(vec![Content::text(text)]));
        }

        let this = self.clone();
        with_hard_timeout(std::time::Duration::from_secs(60), "read_page", async move {
            let bm = this.get_browser().await?;
            let tab = acquire_tab(&bm).await?;

            let timed_result = tokio::time::timeout(
                FETCH_HARD_TIMEOUT,
                navigate_and_fetch_html(tab.page(), &params.url),
            ).await;
            tab.close().await;

            let html = match timed_result {
                Ok(Ok(html)) => html,
                Ok(Err(e)) => {
                    if is_fatal_cdp_error_anyhow(&e) { bm.mark_unhealthy(); }
                    return Err(to_mcp_error(e.to_string()));
                }
                Err(_) => {
                    tracing::error!(url = %params.url, "read_page hard timeout — CDP may be unresponsive");
                    bm.mark_unhealthy();
                    return Err(to_mcp_error("Page read timed out. Browser connection may be lost — it will auto-reconnect on next call."));
                }
            };

            if html.is_empty() {
                return Err(to_mcp_error(format!(
                    "Failed to load content from {} — page returned empty HTML", params.url
                )));
            }

            let extracted = crate::extract::content::ContentExtractor::extract(
                &html, &params.url, max_length,
            ).map_err(to_mcp_error)?;

            if extracted.is_low_quality() {
                let reason = extracted.low_quality_reason().unwrap_or("unknown");
                let text = format!(
                    "# {}\n\nSource: {} [READ_FAILED]\n\n---\n\n{}\n\n> Reason: {} (quality: {:.2})",
                    extracted.title, params.url, extracted.content, reason, extracted.quality
                );
                return Ok(CallToolResult::success(vec![Content::text(text)]));
            }

            let mut content = extracted.content;
            if !params.include_links {
                content = crate::extract::content::strip_markdown_links(&content);
            }

            this.cache.insert(
                cache_key,
                CachedContent { title: extracted.title.clone(), content: content.clone() },
            ).await;

            let text = format!(
                "# {}\n\nSource: {}\n\n---\n\n{}",
                extracted.title, params.url, content
            );
            Ok(CallToolResult::success(vec![Content::text(text)]))
        }).await
    }

    #[tool(description = "Read multiple URLs concurrently using Chrome and extract their content as clean Markdown. Supports up to 10 URLs per call with configurable concurrency. Returns a summary of successful reads and any errors.")]
    async fn batch_read(
        &self,
        Parameters(params): Parameters<tools::BatchReadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let urls: Vec<String> = params.urls.into_iter().take(10).collect();
        for url in &urls {
            interaction::validate_url(url, self.allow_private_urls).map_err(to_mcp_error)?;
        }
        if urls.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No URLs provided.")]));
        }

        let this = self.clone();
        with_hard_timeout(std::time::Duration::from_secs(75), "batch_read", async move {
            let max_length_per_page = params.max_length_per_page.clamp(1, 15000);
            let bm = this.get_browser().await?.clone();
            let effective_concurrency = params.concurrency
                .clamp(1, 10)
                .min(bm.tab_pool().max_tabs())
                .min(urls.len());

            let total = urls.len();
            let (results, timed_out) = read_urls_concurrent(
                bm, &urls, max_length_per_page, effective_concurrency, this.cache.clone(),
            ).await;

            if timed_out && results.is_empty() {
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "batch_read timed out after 60s ({} URLs)", total
                ))]));
            }

            let mut successes = vec![];
            let mut errors = vec![];
            for (url, result) in results {
                match result {
                    Ok(ex) => successes.push((url, format!("# {}\n\n{}", ex.title, ex.content))),
                    Err(e) => errors.push((url, format!("{:#}", e))),
                }
            }

            let mut output = format!("## Successfully read {}/{} pages\n\n", successes.len(), total);
            for (i, (url, content)) in successes.iter().enumerate() {
                output.push_str(&format!("### [{}] {} \n\n{}\n\n---\n\n", i + 1, url, content));
            }
            if !errors.is_empty() {
                output.push_str(&format!("## Errors ({})\n\n", errors.len()));
                for (url, err) in &errors {
                    output.push_str(&format!("- {}: {}\n", url, err));
                }
            }

            Ok(CallToolResult::success(vec![Content::text(output)]))
        }).await
    }

    #[tool(description = "RECOMMENDED: Search the web and automatically read the top results in one call. Returns both search result list and extracted page content. Use this as your primary research tool — it replaces the need for separate web_search + read_page calls.")]
    async fn search_and_read(
        &self,
        Parameters(params): Parameters<tools::SearchAndReadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let query = params.query.trim().to_string();
        if query.is_empty() || query.chars().count() > 500 {
            return Err(to_mcp_error("Query must be 1-500 characters"));
        }
        let this = self.clone();
        with_hard_timeout(std::time::Duration::from_secs(90), "search_and_read", async move {
            let search_count = params.search_count.clamp(1, 20);
            let max_length_per_page = params.max_length_per_page.clamp(1, 15000);

            let bm = this.get_browser().await?;
            let rl = this.rate_limiter(&bm).await;
            rl.wait().await;

            let engine = select_engine(
                &params.engine,
                bm.mode(),
                &this.default_engine,
                this.region,
            );

            let tab = acquire_tab(&bm).await?;
            let search_result = async {
                search_with_fallback(
                    engine,
                    bm.mode(),
                    tab.page(),
                    &query,
                    search_count,
                    this.region,
                ).await
            }.await;
            tab.close().await;

            let search_cdp_check: Result<(), ErrorData> = search_result
                .as_ref()
                .map(|_| ())
                .map_err(|e| to_mcp_error(e.to_string()));
            this.check_cdp_error(&search_cdp_check, &bm);

            if search_result.is_err() {
                rl.backoff().await;
            } else {
                rl.reset_penalty().await;
            }

            let (engine_name, search_results) = search_result.map_err(to_mcp_error)?;

            let read_count = params.read_count.clamp(1, 5).min(search_results.len());
            let mut skipped_urls = vec![];
            let urls_to_read: Vec<String> = search_results
                .iter()
                .take(read_count)
                .filter(|r| {
                    if interaction::validate_url(&r.url, this.allow_private_urls).is_ok() {
                        true
                    } else {
                        tracing::debug!(url = %r.url, "Skipping invalid URL in search_and_read");
                        skipped_urls.push(r.url.clone());
                        false
                    }
                })
                .map(|r| r.url.clone())
                .collect();

            let concurrency = read_count.min(bm.tab_pool().max_tabs());
            let (read_results, read_timed_out) = read_urls_concurrent(
                bm, &urls_to_read, max_length_per_page, concurrency, this.cache.clone(),
            ).await;

            let mut read_content: HashMap<String, String> = HashMap::new();
            let mut read_errors: Vec<(String, String)> = Vec::new();
            for (url, result) in read_results {
                match result {
                    Ok(ex) => { read_content.insert(url, ex.content); }
                    Err(e) => { read_errors.push((url, format!("{:#}", e))); }
                }
            }

            let mut output = format!("## Search results for \"{}\" (via {})\n\n", query, engine_name);
            for (i, r) in search_results.iter().enumerate() {
                let read_marker = if read_content.contains_key(&r.url) { " ⭐ read" } else { "" };
                output.push_str(&format!("{}. [{}]({}){}\n   {}\n\n", i + 1, r.title, r.url, read_marker, r.snippet));
            }

            if !read_content.is_empty() {
                output.push_str("---\n\n## Full content\n\n");
                for (i, r) in search_results.iter().take(read_count).enumerate() {
                    if let Some(content) = read_content.get(&r.url) {
                        output.push_str(&format!("### [{}] {}\nSource: {}\n\n{}\n\n---\n\n", i + 1, r.title, r.url, content));
                    }
                }
            }

            if !read_errors.is_empty() {
                output.push_str(&format!("\n## Read Errors ({})\n\n", read_errors.len()));
                for (url, err) in &read_errors {
                    output.push_str(&format!("- {}: {}\n", url, err));
                }
            }

            if !skipped_urls.is_empty() {
                output.push_str(&format!("\n## Skipped URLs ({})\n\n", skipped_urls.len()));
                for url in &skipped_urls {
                    output.push_str(&format!("- {}: invalid or private URL\n", url));
                }
            }

            if read_timed_out {
                output.push_str("\n> ⚠️ Page reading timed out after 60s. Some results may be missing.\n");
            }

            Ok(CallToolResult::success(vec![Content::text(output)]))
        }).await
    }

    #[tool(description = "Take a screenshot of a webpage. Only use when visual content is specifically needed — prefer read_page for text content as it is far more token-efficient.")]
    async fn screenshot(
        &self,
        Parameters(params): Parameters<tools::ScreenshotParams>,
    ) -> Result<CallToolResult, ErrorData> {
        interaction::validate_url(&params.url, self.allow_private_urls).map_err(to_mcp_error)?;
        if let Some(ref path) = params.file_path {
            interaction::validate_file_path(path).map_err(to_mcp_error)?;
        }

        let this = self.clone();
        with_hard_timeout(std::time::Duration::from_secs(60), "screenshot", async move {
            let bm = this.get_browser().await?;
            let tab = acquire_tab(&bm).await?;
            let file_path = params.file_path.clone();
            let format_str = params.format.clone();

            let cdp_work = async {
                interaction::navigate(tab.page(), &params.url, 15).await.map_err(to_mcp_error)?;

                let data = match format_str.as_str() {
                    "jpeg" | "jpg" => {
                        tab.page().screenshot_jpeg(85).await
                            .map_err(|e| to_mcp_error(format!("Screenshot failed: {}", e)))?
                    }
                    _ => {
                        tab.page().screenshot().await
                            .map_err(|e| to_mcp_error(format!("Screenshot failed: {}", e)))?
                    }
                };
                Ok::<Vec<u8>, ErrorData>(data)
            };

            let timed_result = tokio::time::timeout(FETCH_HARD_TIMEOUT, cdp_work).await;
            tab.close().await;

            let data = match timed_result {
                Ok(inner) => {
                    this.check_cdp_error(&inner, &bm);
                    inner?
                }
                Err(_) => {
                    tracing::error!(url = %params.url, "screenshot hard timeout");
                    bm.mark_unhealthy();
                    return Err(to_mcp_error("Screenshot timed out. Browser connection may be lost."));
                }
            };

            if let Some(ref path) = file_path {
                std::fs::write(path, &data)
                    .map_err(|e| to_mcp_error(format!("Failed to write file: {e}")))?;
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Screenshot saved to {} ({} bytes)", path, data.len()
                ))]))
            } else {
                use base64::Engine as _;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                let mime = match format_str.as_str() {
                    "jpeg" | "jpg" => "image/jpeg",
                    _ => "image/png",
                };
                Ok(CallToolResult::success(vec![Content::image(b64, mime)]))
            }
        }).await
    }

    #[tool(description = "Detect and click OAuth/SSO authorization flows (SSO buttons, consent pages, SAML, popups, multi-step redirects). \
        Use when read_page returns [READ_FAILED] on OAuth/SSO pages — not expired cookies (use sync_login). \
        Returning Google users: Chrome FedCM auto-reauthn may complete after the tool triggers the flow. \
        Limitation (CDP): cannot interact with first-time Google FedCM account picker — user must complete initial Google auth manually in Chrome once. \
        Does NOT handle: username/password login, CAPTCHA, or multi-factor authentication.")]
    async fn click_authorize(
        &self,
        Parameters(params): Parameters<tools::ClickAuthorizeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let this = self.clone();
        let hard_timeout = std::time::Duration::from_secs(params.timeout + 15);
        with_hard_timeout(hard_timeout, "click_authorize", async move {
            let bm = this.get_browser().await?;
            authorize::handle_click_authorize(bm, params, this.allow_private_urls).await
        }).await
    }

    #[tool(description = "General-purpose popup/new-tab handler. Use when a page action opens a new tab that needs interaction. \
        THREE modes: (1) Auth popups — auto-detects Google account chooser, OAuth consent, SAML and handles them (use preferred_account to pick a specific Google account). \
        (2) Non-auth popups — use popup_click to click a specific button/element in the popup (e.g. confirm dialogs, consent buttons). \
        (3) Observe — omit popup_click to just detect the popup and return its content preview. \
        When to use handle_popup vs click_authorize: use click_authorize for full SSO flows (one call does detect → click SSO button → handle popup → return). \
        Use handle_popup when you need fine-grained control: custom trigger element, specific account selection, or non-auth popup interaction. \
        Does NOT handle: browser-native FedCM dialogs, username/password forms, CAPTCHA.")]
    async fn handle_popup(
        &self,
        Parameters(params): Parameters<tools::HandlePopupParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let this = self.clone();
        let hard_timeout = std::time::Duration::from_secs(params.timeout + 15);
        with_hard_timeout(hard_timeout, "handle_popup", async move {
            let bm = this.get_browser().await?;
            popup_handler::handle_popup(bm, params, this.allow_private_urls).await
        }).await
    }

    #[tool(description = "Sync login state (cookies, sessions) from user's main Chrome to the debug profile. \
        UserChrome mode only (requires setup); not needed in AutoConnect. \
        Cannot transfer Google OAuth sessions (Chrome cookie encryption) — use click_authorize or manual Google sign-in in the debug profile. \
        Use when read_page returns [READ_FAILED] due to expired cookies/sessions (non-OAuth). \
        After syncing, the browser reconnects automatically — retry the failed read_page call.")]
    async fn sync_login(&self) -> Result<CallToolResult, ErrorData> {
        // Disconnect cleanly first, then kill Chrome to avoid broken WebSocket propagation
        self.browser.shutdown().await;
        self.kill_debug_chrome().await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let (synced, skipped) = crate::browser::profile::sync_login_files()
            .map_err(to_mcp_error)?;

        let mut msg = format!(
            "Login state synced: {} files updated, {} skipped.\n",
            synced, skipped
        );
        if skipped > 0 {
            msg.push_str("Some files were locked — they will be available after Chrome restarts.\n");
        }
        msg.push_str("Browser will auto-restart and reconnect on next tool call. Retry your previous request now.");

        Ok(CallToolResult::success(vec![Content::text(msg)]))
    }
}

#[tool_handler]
impl ServerHandler for SearchServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::new(
                "ailonk-search",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "ailonk-search: Web search and page reading via real Chrome browser with anti-bot protection.\n\
                \n\
                TOOL SELECTION:\n\
                1. search_and_read — primary tool. Searches AND reads top results in one call.\n\
                2. web_search — when you only need titles/URLs/snippets, no full content.\n\
                3. read_page — read a specific URL you already have (max_length up to 15000).\n\
                4. batch_read — read multiple known URLs concurrently (up to 10).\n\
                5. screenshot — visual capture only; prefer read_page for text.\n\
                6. click_authorize — handle OAuth/SSO flows (SSO buttons, consent pages, SAML, popups, multi-step redirects). \
                Use when read_page fails on OAuth/SSO pages. Supports preferred_account param to select a specific Google account. \
                Returning Google users: FedCM auto-reauthn may complete after tool triggers the flow. \
                Limitation: cannot handle first-time Google FedCM account picker (CDP) — user must manually authorize Google once in Chrome first.\n\
                7. handle_popup — general-purpose popup/new-tab handler. Three modes: auth auto-handle, non-auth click (popup_click), \
                or observe-only. Use when you need control over popup interaction (custom trigger, specific account, non-auth popups). \
                For full SSO automation prefer click_authorize.\n\
                8. sync_login — refresh login state from user's Chrome when pages return [READ_FAILED] due to expired cookies/sessions. \
                UserChrome mode only (not needed in AutoConnect). Cannot sync Google OAuth sessions (Chrome cookie encryption).\n\
                \n\
                QUERY CRAFTING:\n\
                - Be specific: include entity names, versions, dates. Good: 'SpaceX IPO 2026 pricing'. Bad: 'SpaceX news'.\n\
                - Match query language to target content: Chinese topics → Chinese keywords, English docs → English keywords.\n\
                - Include the current year for time-sensitive queries (e.g. '2026 AI model benchmark').\n\
                - One intent per query — never combine unrelated subtopics in a single search.\n\
                - Use search operators when needed: \"exact phrase\", site:domain.com, -exclude_term.\n\
                - Max query length: 500 characters.\n\
                \n\
                SEARCH STRATEGY:\n\
                - For multi-topic queries (e.g. comparing 4 companies), split into separate searches per topic.\n\
                - For deep research, use multiple rounds: first broad, then targeted follow-ups with refined keywords.\n\
                - Cross-verify key facts across rounds for accuracy.\n\
                \n\
                ENGINE SELECTION:\n\
                - auto (default): best engine based on region. Usually optimal.\n\
                - bing: stable for Chinese content, use when Google is unreliable.\n\
                - google: best for English technical docs and global content.\n\
                - duckduckgo: for privacy-sensitive queries or when other engines hit CAPTCHAs.\n\
                \n\
                PARAMETER GUIDE:\n\
                - read_count: 1-2 for quick overview, 3 for standard research, 4-5 for deep investigation.\n\
                - max_length_per_page: default 5000. Increase to 8000-15000 for long-form articles, \
                financial reports, or technical documentation.\n\
                - search_count: default 10, increase for broader coverage.\n\
                \n\
                CONTENT RELIABILITY:\n\
                - Pages marked [READ_FAILED] returned low-quality content (CAPTCHA, login wall, empty).\n\
                - When you see [READ_FAILED], try an alternative URL from search results instead of retrying the same one.\n\
                - Avoid reading pages from known walled sites (e.g. tieba.baidu.com, zhihu.com/question without answer) \
                when better alternatives exist.\n\
                - If a page requires OAuth/SSO authorization, call click_authorize first, then retry read_page. \
                For first-time Google auth, user must manually complete FedCM account selection in Chrome once — CDP cannot interact with it.\n\
                - If a page requires login (expired cookies/sessions), call sync_login to refresh login state, then retry. \
                UserChrome mode only; not needed in AutoConnect. sync_login cannot transfer Google OAuth sessions — use click_authorize or manual Google sign-in.\n\
                \n\
                NOTES:\n\
                - Results are clean Markdown, no summarization — you interpret the content.\n\
                - The browser handles JavaScript rendering, cookie consent, and anti-bot measures.\n\
                - If a call fails with a connection error, retry once — the browser auto-reconnects.",
            )
    }
}

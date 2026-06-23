pub mod tools;

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

    async fn browser(&self) -> Result<Arc<BrowserManager>, ErrorData> {
        self.browser.get().await.map_err(to_mcp_error)
    }

    /// Kill the debug Chrome process on port 19222 so cookie files can be safely overwritten.
    async fn kill_debug_chrome(&self) {
        #[cfg(unix)]
        {
            let output = tokio::process::Command::new("lsof")
                .args(["-ti", ":19222"])
                .output()
                .await;
            if let Ok(out) = output {
                let pids = String::from_utf8_lossy(&out.stdout);
                for pid_str in pids.lines() {
                    let pid = pid_str.trim();
                    if !pid.is_empty() {
                        let _ = tokio::process::Command::new("kill")
                            .args(["-TERM", pid])
                            .output()
                            .await;
                        tracing::info!(pid, "Sent SIGTERM to debug Chrome");
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            let _ = tokio::process::Command::new("taskkill")
                .args(["/F", "/FI", "WINDOWTITLE eq *--remote-debugging-port=19222*"])
                .output()
                .await;
        }
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

fn is_fatal_cdp_error_anyhow(err: &anyhow::Error) -> bool {
    is_fatal_cdp_error(&err.to_string())
}

/// Shared read logic: acquire tab → navigate → CAPTCHA → extract content.
/// Used by batch_read and search_and_read to eliminate code duplication.
async fn fetch_and_extract(
    bm: &BrowserManager,
    url: &str,
    max_len: usize,
    cache: &ContentCache,
) -> anyhow::Result<crate::extract::content::ExtractedContent> {
    let tab = bm.tab_pool().acquire().await?;
    let page_result = async {
        interaction::navigate(tab.page(), url, 15).await?;
        if interaction::is_captcha_present(tab.page()).await {
            tracing::warn!(url = %url, "CAPTCHA detected");
            match interaction::resolve_captcha_loop(tab.page(), 1).await {
                Ok(_) => {
                    let _ = tab.page().wait_for_network_idle(500, 3000).await;
                }
                Err(_) => anyhow::bail!("[READ_FAILED] CAPTCHA unresolved"),
            }
        }
        tab.page().content().await
            .map_err(|e| anyhow::anyhow!("Content fetch failed: {}", e))
    }.await;
    tab.close().await;
    if let Err(ref e) = page_result {
        if is_fatal_cdp_error_anyhow(e) {
            bm.mark_unhealthy();
        }
    }
    let html = page_result?;
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
        let count = params.count.clamp(1, 20);
        let bm = self.browser().await?;
        let rl = self.rate_limiter(&bm).await;
        rl.wait().await;

        let engine = select_engine(
            &params.engine,
            bm.mode(),
            &self.default_engine,
            self.region,
        );

        let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;

        let result = async {
            search_with_fallback(
                engine,
                bm.mode(),
                tab.page(),
                &query,
                count,
                self.region,
            )
            .await
            .map_err(to_mcp_error)
        }
        .await;

        tab.close().await;
        self.check_cdp_error(&result, &bm);

        match &result {
            Ok(_) => rl.reset_penalty().await,
            Err(_) => rl.backoff().await,
        }

        let (engine_name, results) = result?;
        let text = format_search_results(&query, &engine_name, &results);
        Ok(CallToolResult::success(vec![Content::text(text)]))
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

        let bm = self.browser().await?;
        let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;

        let nav_result = interaction::navigate(tab.page(), &params.url, 15).await
            .map_err(to_mcp_error);
        if let Err(ref e) = nav_result {
            if is_fatal_cdp_error(&e.message) {
                bm.mark_unhealthy();
            }
            tab.close().await;
            return Err(nav_result.unwrap_err());
        }

        if interaction::is_captcha_present(tab.page()).await {
            tracing::warn!(url = %params.url, "CAPTCHA detected, attempting resolve...");
            match interaction::resolve_captcha_loop(tab.page(), 1).await {
                Ok(_) => {
                    tracing::info!("CAPTCHA resolved");
                    let _ = tab.page().wait_for_network_idle(500, 3000).await;
                }
                Err(_) => {
                    tracing::warn!("CAPTCHA could not be resolved");
                    tab.close().await;
                    let text = format!(
                        "# CAPTCHA\n\nSource: {} [READ_FAILED]\n\n---\n\n> Reason: CAPTCHA unresolved",
                        params.url
                    );
                    return Ok(CallToolResult::success(vec![Content::text(text)]));
                }
            }
        }

        let result = tab.page().content().await
            .map_err(|e| to_mcp_error(format!("Failed to get page content: {}", e)));
        tab.close().await;
        self.check_cdp_error(&result, &bm);
        let html = result?;

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

        self.cache.insert(
            cache_key,
            CachedContent { title: extracted.title.clone(), content: content.clone() },
        ).await;

        let text = format!(
            "# {}\n\nSource: {}\n\n---\n\n{}",
            extracted.title, params.url, content
        );
        Ok(CallToolResult::success(vec![Content::text(text)]))
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

        let max_length_per_page = params.max_length_per_page.clamp(1, 15000);
        let bm = self.browser().await?.clone();
        let effective_concurrency = params.concurrency
            .clamp(1, 10)
            .min(bm.tab_pool().max_tabs())
            .min(urls.len());

        let semaphore = Arc::new(tokio::sync::Semaphore::new(effective_concurrency));
        let mut handles = vec![];

        for url in urls.iter() {
            let sem = semaphore.clone();
            let url = url.clone();
            let bm = bm.clone();
            let cache = self.cache.clone();
            let max_len = max_length_per_page;

            handles.push(tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => return (url, Err(anyhow::anyhow!("Semaphore error: {e}"))),
                };

                let cache_key = ContentCache::key(&url, true, max_len);
                if let Some(cached) = cache.get(&cache_key).await {
                    return (url, Ok(format!("# {}\n\n{}", cached.title, cached.content)));
                }

                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    fetch_and_extract(&bm, &url, max_len, &cache),
                )
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Page read timeout after 30s")))
                .map(|ex| format!("# {}\n\n{}", ex.title, ex.content));

                (url, result)
            }));
        }

        let total = urls.len();
        let results = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            futures::future::join_all(&mut handles),
        ).await {
            Ok(r) => r,
            Err(_) => {
                for h in handles {
                    h.abort();
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                return Ok(CallToolResult::success(vec![Content::text(format!(
                    "batch_read timed out after 60s ({} URLs)", total
                ))]));
            }
        };

        let mut successes = vec![];
        let mut errors = vec![];
        for result in results {
            match result {
                Ok((url, Ok(content))) => successes.push((url, content)),
                Ok((url, Err(e))) => errors.push((url, format!("{:#}", e))),
                Err(e) => errors.push(("unknown".to_string(), format!("Task panic: {e}"))),
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
        let search_count = params.search_count.clamp(1, 20);
        let max_length_per_page = params.max_length_per_page.clamp(1, 15000);

        let bm = self.browser().await?;
        let rl = self.rate_limiter(&bm).await;
        rl.wait().await;

        let engine = select_engine(
            &params.engine,
            bm.mode(),
            &self.default_engine,
            self.region,
        );

        let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;
        let search_result = async {
            search_with_fallback(
                engine,
                bm.mode(),
                tab.page(),
                &query,
                search_count,
                self.region,
            ).await
        }.await;
        tab.close().await;

        let search_cdp_check: Result<(), ErrorData> = search_result
            .as_ref()
            .map(|_| ())
            .map_err(|e| to_mcp_error(e.to_string()));
        self.check_cdp_error(&search_cdp_check, &bm);

        if search_result.is_err() {
            rl.backoff().await;
        } else {
            rl.reset_penalty().await;
        }

        let (engine_name, search_results) = search_result.map_err(to_mcp_error)?;

        let read_count = params.read_count.clamp(1, 5).min(search_results.len());
        let urls_to_read: Vec<String> = search_results
            .iter()
            .take(read_count)
            .filter(|r| interaction::validate_url(&r.url, self.allow_private_urls).is_ok())
            .map(|r| r.url.clone())
            .collect();

        let bm = bm.clone();
        let read_semaphore = Arc::new(tokio::sync::Semaphore::new(
            read_count.min(bm.tab_pool().max_tabs()),
        ));
        let mut handles = vec![];
        for url in urls_to_read.iter() {
            let sem = read_semaphore.clone();
            let url = url.clone();
            let bm = bm.clone();
            let cache = self.cache.clone();
            let max_len = max_length_per_page;

            handles.push(tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => return (url, Err::<String, anyhow::Error>(anyhow::anyhow!("Semaphore error: {e}"))),
                };

                let cache_key = ContentCache::key(&url, true, max_len);
                if let Some(cached) = cache.get(&cache_key).await {
                    return (url, Ok::<String, anyhow::Error>(cached.content));
                }

                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    fetch_and_extract(&bm, &url, max_len, &cache),
                )
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Page read timeout after 30s")))
                .map(|ex| ex.content);

                (url, result)
            }));
        }

        let (read_results, read_timed_out) = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            futures::future::join_all(&mut handles),
        ).await {
            Ok(r) => (r, false),
            Err(_) => {
                for h in &handles { h.abort(); }
                (vec![], true)
            }
        };

        let mut read_content: HashMap<String, String> = HashMap::new();
        let mut read_errors: Vec<(String, String)> = Vec::new();
        for result in read_results {
            match result {
                Ok((url, Ok(content))) => { read_content.insert(url, content); }
                Ok((url, Err(e))) => { read_errors.push((url, format!("{:#}", e))); }
                Err(e) => { read_errors.push(("unknown".to_string(), format!("Task error: {e}"))); }
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

        if read_timed_out {
            output.push_str("\n> ⚠️ Page reading timed out after 60s. Some results may be missing.\n");
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
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

        let bm = self.browser().await?;
        let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;
        let file_path = params.file_path.clone();
        let format_str = params.format.clone();

        let result = async {
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
        }.await;

        tab.close().await;
        self.check_cdp_error(&result, &bm);
        let data = result?;

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
    }

    #[tool(description = "Sync login state (cookies, sessions) from user's main Chrome to the debug profile. Use when read_page returns [READ_FAILED] on a page that requires authentication, or when the user reports expired login. After syncing, the browser will reconnect automatically — retry the failed read_page call.")]
    async fn sync_login(&self) -> Result<CallToolResult, ErrorData> {
        // Kill the debug Chrome process so cookie files are unlocked and
        // won't be overwritten by Chrome's shutdown flush.
        self.kill_debug_chrome().await;
        self.browser.shutdown().await;

        // Brief wait for process to fully exit and release file locks
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
                \n\
                SEARCH STRATEGY:\n\
                - For multi-topic queries (e.g. comparing 4 companies), split into separate searches per topic. \
                Broad queries tend to be dominated by one subtopic.\n\
                - For deep research, use multiple rounds: first broad, then targeted follow-ups with refined keywords.\n\
                - Cross-verify key facts across rounds for accuracy.\n\
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
                - If a page requires login and returns empty/error content, call sync_login to refresh \
                the browser's login state from the user's main Chrome, then retry.\n\
                \n\
                NOTES:\n\
                - Results are clean Markdown, no summarization — you interpret the content.\n\
                - The browser handles JavaScript rendering, cookie consent, and anti-bot measures.\n\
                - If a call fails with a connection error, retry once — the browser auto-reconnects.",
            )
    }
}

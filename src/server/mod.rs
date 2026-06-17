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
use crate::browser::manager::BrowserManager;
use crate::cache::{CachedContent, ContentCache};
use crate::search::engine::{
    format_search_results, search_with_fallback, select_engine, to_mcp_error, RateLimiter,
};

#[derive(Clone)]
pub struct SearchServer {
    pub browser_manager: Arc<BrowserManager>,
    pub default_engine: String,
    pub region: &'static str,
    pub rate_limiter: Arc<RateLimiter>,
    pub cache: ContentCache,
}

impl SearchServer {
    pub fn new(
        browser_manager: Arc<BrowserManager>,
        default_engine: String,
        cache_ttl: u64,
        region: &'static str,
    ) -> Self {
        use crate::browser::manager::ConnectionMode;
        let rate_interval = match browser_manager.mode() {
            ConnectionMode::Headless => 5000,
            ConnectionMode::UserChrome => 2000,
        };
        Self {
            browser_manager,
            default_engine,
            region,
            rate_limiter: Arc::new(RateLimiter::new(rate_interval)),
            cache: ContentCache::new(cache_ttl),
        }
    }
}

#[tool_router]
impl SearchServer {
    #[tool(description = "Search the web using a real Chrome browser with anti-bot protection. Returns a list of results with titles, URLs, and snippets. Use this when you need search results but don't need to read the full page content. Supports Google, Bing, and DuckDuckGo.")]
    async fn web_search(
        &self,
        Parameters(params): Parameters<tools::WebSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.rate_limiter.wait().await;

        let engine = select_engine(
            &params.engine,
            self.browser_manager.mode(),
            &self.default_engine,
            self.region,
        );

        let tab = self.browser_manager.tab_pool().acquire().await.map_err(to_mcp_error)?;

        let result = async {
            search_with_fallback(
                engine,
                self.browser_manager.mode(),
                tab.page(),
                &params.query,
                params.count,
                self.region,
            )
            .await
            .map_err(to_mcp_error)
        }
        .await;

        tab.close().await;

        match &result {
            Ok(_) => self.rate_limiter.reset_penalty().await,
            Err(_) => self.rate_limiter.backoff().await,
        }

        let (engine_name, results) = result?;
        let text = format_search_results(&params.query, &engine_name, &results);
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Fetch a single URL using Chrome and extract the main content as clean Markdown. Handles JavaScript-rendered pages and cookie consent. Use this when you have a specific URL and need its content.")]
    async fn read_page(
        &self,
        Parameters(params): Parameters<tools::ReadPageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        interaction::validate_url(&params.url).map_err(to_mcp_error)?;
        let cache_key = ContentCache::key(&params.url, params.include_links, params.max_length);
        if let Some(cached) = self.cache.get(&cache_key).await {
            tracing::debug!(url = %params.url, "Cache hit");
            let text = format!(
                "# {}\n\nSource: {} (cached)\n\n---\n\n{}",
                cached.title, params.url, cached.content
            );
            return Ok(CallToolResult::success(vec![Content::text(text)]));
        }

        let tab = self.browser_manager.tab_pool().acquire().await.map_err(to_mcp_error)?;

        let result = async {
            interaction::navigate(tab.page(), &params.url, 15).await.map_err(to_mcp_error)?;
            tab.page().wait_for("body", 5000).await.ok();
            tab.page().wait(1000).await;
            tab.page().content().await
                .map_err(|e| to_mcp_error(format!("Failed to get page content: {}", e)))
        }
        .await;

        tab.close().await;
        let html = result?;

        if html.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(format!(
                "Failed to load content from {}", params.url
            ))]));
        }

        let extracted = crate::extract::content::ContentExtractor::extract(
            &html, &params.url, params.max_length,
        ).map_err(to_mcp_error)?;

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
            interaction::validate_url(url).map_err(to_mcp_error)?;
        }
        if urls.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No URLs provided.")]));
        }

        let effective_concurrency = params.concurrency
            .max(1)
            .min(self.browser_manager.tab_pool().max_tabs())
            .min(urls.len());

        let semaphore = Arc::new(tokio::sync::Semaphore::new(effective_concurrency));
        let mut handles = vec![];

        for url in urls.iter() {
            let sem = semaphore.clone();
            let url = url.clone();
            let bm = self.browser_manager.clone();
            let cache = self.cache.clone();
            let max_length = params.max_length_per_page;

            handles.push(tokio::spawn(async move {
                let _permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(e) => return (url, Err(anyhow::anyhow!("Semaphore error: {e}"))),
                };

                let cache_key = ContentCache::key(&url, true, max_length);
                if let Some(cached) = cache.get(&cache_key).await {
                    return (url, Ok(format!("# {}\n\n{}", cached.title, cached.content)));
                }

                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    async {
                        let tab = bm.tab_pool().acquire().await?;
                        let page_result = async {
                            interaction::navigate(tab.page(), &url, 15).await?;
                            tab.page().wait_for("body", 5000).await.ok();
                            tab.page().wait(1000).await;
                            tab.page().content().await
                                .map_err(|e| anyhow::anyhow!("Content fetch failed: {}", e))
                        }.await;
                        tab.close().await;
                        let html = page_result?;

                        if html.is_empty() {
                            anyhow::bail!("Empty content");
                        }

                        let extracted = crate::extract::content::ContentExtractor::extract(
                            &html, &url, max_length,
                        )?;

                        cache.insert(
                            cache_key,
                            CachedContent {
                                title: extracted.title.clone(),
                                content: extracted.content.clone(),
                            },
                        ).await;

                        Ok::<String, anyhow::Error>(format!("# {}\n\n{}", extracted.title, extracted.content))
                    },
                )
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Page read timeout after 30s")));

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

    #[tool(description = "Search the web and automatically read the top results. Combines web_search + batch_read in a single call — optimal for AI research tasks. Returns both search result list and full content of top pages.")]
    async fn search_and_read(
        &self,
        Parameters(params): Parameters<tools::SearchAndReadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.rate_limiter.wait().await;
        let engine = select_engine(
            &params.engine,
            self.browser_manager.mode(),
            &self.default_engine,
            self.region,
        );

        let tab = self.browser_manager.tab_pool().acquire().await.map_err(to_mcp_error)?;
        let search_result = async {
            search_with_fallback(
                engine,
                self.browser_manager.mode(),
                tab.page(),
                &params.query,
                params.search_count,
                self.region,
            ).await
        }.await;
        tab.close().await;

        if search_result.is_err() {
            self.rate_limiter.backoff().await;
        } else {
            self.rate_limiter.reset_penalty().await;
        }

        let (engine_name, search_results) = search_result.map_err(to_mcp_error)?;

        let read_count = params.read_count.min(5).min(search_results.len());
        let urls_to_read: Vec<String> = search_results.iter().take(read_count).map(|r| r.url.clone()).collect();

        let mut handles = vec![];
        for url in urls_to_read.iter() {
            let url = url.clone();
            let bm = self.browser_manager.clone();
            let cache = self.cache.clone();
            let max_length = params.max_length_per_page;

            handles.push(tokio::spawn(async move {
                let cache_key = ContentCache::key(&url, true, max_length);
                if let Some(cached) = cache.get(&cache_key).await {
                    return (url, Ok::<String, anyhow::Error>(cached.content));
                }

                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    async {
                        let tab = bm.tab_pool().acquire().await?;
                        let page_result = async {
                            interaction::navigate(tab.page(), &url, 15).await?;
                            tab.page().wait_for("body", 5000).await.ok();
                            tab.page().wait(1000).await;
                            tab.page().content().await
                                .map_err(|e| anyhow::anyhow!("Content fetch failed: {}", e))
                        }.await;
                        tab.close().await;
                        let html = page_result?;

                        let extracted = crate::extract::content::ContentExtractor::extract(
                            &html, &url, max_length,
                        )?;

                        cache.insert(
                            cache_key,
                            CachedContent {
                                title: extracted.title.clone(),
                                content: extracted.content.clone(),
                            },
                        ).await;

                        Ok(extracted.content)
                    },
                )
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("Page read timeout after 30s")));

                (url, result)
            }));
        }

        let read_results = match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            futures::future::join_all(&mut handles),
        ).await {
            Ok(r) => r,
            Err(_) => {
                for h in &handles { h.abort(); }
                vec![]
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

        let mut output = format!("## Search results for \"{}\" (via {})\n\n", params.query, engine_name);
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

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    #[tool(description = "Take a screenshot of a webpage using Chrome. Returns base64-encoded PNG image or saves to file.")]
    async fn screenshot(
        &self,
        Parameters(params): Parameters<tools::ScreenshotParams>,
    ) -> Result<CallToolResult, ErrorData> {
        interaction::validate_url(&params.url).map_err(to_mcp_error)?;
        if let Some(ref path) = params.file_path {
            interaction::validate_file_path(path).map_err(to_mcp_error)?;
        }

        let tab = self.browser_manager.tab_pool().acquire().await.map_err(to_mcp_error)?;
        let file_path = params.file_path.clone();
        let format_str = params.format.clone();

        let result = async {
            interaction::navigate(tab.page(), &params.url, 15).await.map_err(to_mcp_error)?;
            tab.page().wait_for("body", 5000).await.ok();
            tab.page().wait(1000).await;

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
                "Chrome Search MCP provides web search and content extraction via Chrome DevTools Protocol. \
                Use 'search_and_read' for research tasks (combines search + read in one call). \
                Use 'web_search' when you only need search results. \
                Use 'read_page' or 'batch_read' to extract content from specific URLs. \
                Use 'screenshot' to capture visual snapshots.",
            )
    }
}

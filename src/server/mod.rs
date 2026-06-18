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

    async fn browser(&self) -> Result<&Arc<BrowserManager>, ErrorData> {
        self.browser.get().await.map_err(to_mcp_error)
    }

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
        let bm = self.browser().await?;
        let rl = self.rate_limiter(bm).await;
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
            Ok(_) => rl.reset_penalty().await,
            Err(_) => rl.backoff().await,
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
        interaction::validate_url(&params.url, self.allow_private_urls).map_err(to_mcp_error)?;
        let cache_key = ContentCache::key(&params.url, params.include_links, params.max_length);
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

        let result = async {
            interaction::navigate(tab.page(), &params.url, 15).await.map_err(to_mcp_error)?;
            tab.page().wait(500).await;
            tab.page().content().await
                .map_err(|e| to_mcp_error(format!("Failed to get page content: {}", e)))
        }
        .await;

        tab.close().await;
        let html = result?;

        if html.is_empty() {
            return Err(to_mcp_error(format!(
                "Failed to load content from {} — page returned empty HTML", params.url
            )));
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
            interaction::validate_url(url, self.allow_private_urls).map_err(to_mcp_error)?;
        }
        if urls.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text("No URLs provided.")]));
        }

        let bm = self.browser().await?.clone();
        let effective_concurrency = params.concurrency
            .max(1)
            .min(bm.tab_pool().max_tabs())
            .min(urls.len());

        let semaphore = Arc::new(tokio::sync::Semaphore::new(effective_concurrency));
        let mut handles = vec![];

        for url in urls.iter() {
            let sem = semaphore.clone();
            let url = url.clone();
            let bm = bm.clone();
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
                            tab.page().wait(500).await;
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

    #[tool(description = "RECOMMENDED: Search the web and automatically read the top results in one call. Returns both search result list and extracted page content. Use this as your primary research tool — it replaces the need for separate web_search + read_page calls.")]
    async fn search_and_read(
        &self,
        Parameters(params): Parameters<tools::SearchAndReadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let bm = self.browser().await?;
        let rl = self.rate_limiter(bm).await;
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
                &params.query,
                params.search_count,
                self.region,
            ).await
        }.await;
        tab.close().await;

        if search_result.is_err() {
            rl.backoff().await;
        } else {
            rl.reset_penalty().await;
        }

        let (engine_name, search_results) = search_result.map_err(to_mcp_error)?;

        let read_count = params.read_count.min(5).min(search_results.len());
        let urls_to_read: Vec<String> = search_results.iter().take(read_count).map(|r| r.url.clone()).collect();

        let bm = bm.clone();
        let mut handles = vec![];
        for url in urls_to_read.iter() {
            let url = url.clone();
            let bm = bm.clone();
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
                            tab.page().wait(500).await;
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
            tab.page().wait(500).await;

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
                "ailonk-search: Web search and page reading via real Chrome browser.\n\
                \n\
                RECOMMENDED WORKFLOW:\n\
                1. Start with 'search_and_read' — it searches AND reads top results in one call (most efficient).\n\
                2. Use 'web_search' only when you need search results without reading pages.\n\
                3. Use 'read_page' to read a specific URL you already have.\n\
                4. Use 'batch_read' to read multiple URLs concurrently.\n\
                \n\
                TIPS:\n\
                - search_and_read saves 2-3 tool calls vs web_search + read_page separately.\n\
                - Results are clean Markdown text, optimized for token efficiency.\n\
                - The browser has anti-bot protection and can access pages requiring JavaScript.",
            )
    }
}

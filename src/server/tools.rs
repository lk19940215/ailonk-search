use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    /// The search query string, e.g. 'Rust async programming'
    pub query: String,
    /// Search engine to use. 'auto' selects best engine based on region (default). Options: auto, google, bing, duckduckgo
    #[serde(default)]
    pub engine: EngineChoice,
    /// Number of search results to return (1-20, default: 10)
    #[serde(default = "default_count")]
    pub count: usize,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    /// Automatically select the best search engine based on region
    #[default]
    Auto,
    /// Use Google search
    Google,
    /// Use Bing search (cn.bing.com for China region)
    Bing,
    /// Use DuckDuckGo search
    Duckduckgo,
}

fn default_count() -> usize { 10 }

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPageParams {
    /// The URL to fetch and extract content from (http/https only)
    pub url: String,
    /// Whether to preserve hyperlinks in the extracted Markdown (default: true)
    #[serde(default = "default_true")]
    pub include_links: bool,
    /// Maximum character length of extracted content (default: 15000)
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

fn default_true() -> bool { true }
fn default_max_length() -> usize { 15000 }
fn default_max_length_per_page() -> usize { 5000 }
fn default_concurrency() -> usize { 5 }
fn default_read_count() -> usize { 3 }

fn default_screenshot_format() -> String {
    "png".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchReadParams {
    /// List of URLs to read concurrently. Maximum 10 URLs per call.
    pub urls: Vec<String>,
    /// Maximum character length of extracted content per page (default: 5000)
    #[serde(default = "default_max_length_per_page")]
    pub max_length_per_page: usize,
    /// Maximum number of concurrent browser tabs (default: 5, max: 10)
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    /// The URL to capture (http/https only)
    pub url: String,
    /// Capture the full scrollable page instead of the visible viewport (default: false)
    #[serde(default)]
    #[allow(dead_code)]
    pub full_page: bool,
    /// CSS selector to screenshot a specific element instead of the full page
    #[allow(dead_code)]
    pub selector: Option<String>,
    /// Image format: png, jpeg, or webp (default: png)
    #[serde(default = "default_screenshot_format")]
    pub format: String,
    /// Optional file path to save the screenshot instead of returning inline base64
    pub file_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchAndReadParams {
    /// The search query string
    pub query: String,
    /// Search engine to use (default: auto)
    #[serde(default)]
    pub engine: EngineChoice,
    /// Number of search results to retrieve (default: 10)
    #[serde(default = "default_count")]
    pub search_count: usize,
    /// Number of top search results to read full content (1-5, default: 3)
    #[serde(default = "default_read_count")]
    pub read_count: usize,
    /// Maximum content length per read page (default: 5000)
    #[serde(default = "default_max_length_per_page")]
    pub max_length_per_page: usize,
}

use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    /// Search query. Be specific: include entity names, versions, years. E.g. 'Rust tokio 2026 tutorial', 'SpaceX IPO 2026 pricing'. Match language to target content. Max 500 chars.
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

impl EngineChoice {
    pub fn from_str(s: &str) -> Self {
        match s {
            "google" => Self::Google,
            "bing" => Self::Bing,
            "duckduckgo" | "ddg" => Self::Duckduckgo,
            _ => Self::Auto,
        }
    }
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
    /// Image format: png or jpeg (default: png)
    #[serde(default = "default_screenshot_format")]
    pub format: String,
    /// Optional file path to save the screenshot instead of returning inline base64
    pub file_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClickAuthorizeParams {
    /// The URL that requires OAuth/SSO authorization. Navigate to this URL and handle any auth pages.
    pub url: String,
    /// Maximum time in seconds to wait for the authorization flow to complete (default: 30)
    #[serde(default = "default_auth_timeout")]
    pub timeout: u64,
    /// Preferred account for account selection (e.g. "user@company.com" or "@company.com").
    /// Falls back to PREFERRED_ACCOUNT env var if not set.
    pub preferred_account: Option<String>,
}

fn default_auth_timeout() -> u64 { 30 }

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HandlePopupParams {
    /// The URL of the page where a popup/new tab is expected.
    pub url: String,
    /// Optional: trigger action to cause the popup. Can be a CSS selector (e.g. "#login-btn")
    /// or button text (e.g. "Sign in with Google"). If omitted, just monitors for popups.
    pub trigger: Option<String>,
    /// Optional: only handle popups whose URL contains this string (e.g. "accounts.google.com").
    pub popup_url_contains: Option<String>,
    /// Optional: for auth popups — preferred account email or domain (e.g. "user@company.com" or "@company.com").
    /// Falls back to PREFERRED_ACCOUNT env var if not set.
    pub preferred_account: Option<String>,
    /// Optional: for non-auth popups — click an element in the popup by CSS selector or text.
    /// Useful for confirm dialogs, consent buttons, etc.
    pub popup_click: Option<String>,
    /// Maximum time in seconds to wait for the popup to appear and complete (default: 30)
    #[serde(default = "default_auth_timeout")]
    pub timeout: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchAndReadParams {
    /// Search query. Be specific: include entity names, versions, years. E.g. 'DeepSeek V4 benchmark results', 'React 19 server components guide'. Match language to target content. Max 500 chars.
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

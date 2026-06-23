use async_trait::async_trait;
use eoka::Page;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::browser::manager::ConnectionMode;
use crate::server::tools::EngineChoice;

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &str;
    async fn search(
        &self,
        page: &Page,
        query: &str,
        count: usize,
    ) -> anyhow::Result<Vec<SearchResult>>;
}

pub fn detect_region(cli_region: &str) -> &'static str {
    if cli_region != "auto" {
        return match cli_region {
            "cn" => "cn",
            "global" => "global",
            other => {
                tracing::warn!(region = other, "Unknown region, defaulting to 'global'");
                "global"
            }
        };
    }
    for var in &["LANG", "LC_ALL", "LANGUAGE"] {
        if let Ok(val) = std::env::var(var) {
            let val_lower = val.to_lowercase();
            if val_lower.starts_with("zh_cn")
                || val_lower.starts_with("zh-cn")
                || val_lower.starts_with("zh_hans")
                || val_lower.starts_with("zh-hans")
            {
                return "cn";
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
        {
            let locale = String::from_utf8_lossy(&output.stdout).to_lowercase();
            if locale.contains("zh_cn") || locale.contains("zh-hans") {
                return "cn";
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "(Get-Culture).Name"])
            .output()
        {
            if output.status.success() {
                let culture = String::from_utf8_lossy(&output.stdout).to_lowercase();
                if culture.starts_with("zh-cn") || culture.starts_with("zh-hans") {
                    return "cn";
                }
            }
        }
    }

    "global"
}

pub fn select_engine(
    choice: &EngineChoice,
    connection_mode: &ConnectionMode,
    cli_default: &str,
    region: &'static str,
) -> Box<dyn SearchEngine> {
    let timeout = match connection_mode {
        ConnectionMode::Headless => 25,
        ConnectionMode::UserChrome => 15,
    };
    let effective = if matches!(choice, EngineChoice::Auto) {
        match cli_default {
            "google" => {
                return Box::new(super::google::GoogleEngine {
                    region,
                    nav_timeout: timeout,
                });
            }
            "bing" => {
                return Box::new(super::bing::BingEngine {
                    region,
                    nav_timeout: timeout,
                });
            }
            "duckduckgo" => {
                return Box::new(super::duckduckgo::DuckDuckGoEngine {
                    nav_timeout: timeout,
                });
            }
            _ => choice,
        }
    } else {
        choice
    };
    match effective {
        EngineChoice::Google => Box::new(super::google::GoogleEngine {
            region,
            nav_timeout: timeout,
        }),
        EngineChoice::Bing => Box::new(super::bing::BingEngine {
            region,
            nav_timeout: timeout,
        }),
        EngineChoice::Duckduckgo => Box::new(super::duckduckgo::DuckDuckGoEngine {
            nav_timeout: timeout,
        }),
        EngineChoice::Auto => match (connection_mode, region) {
            (ConnectionMode::UserChrome, "cn") => Box::new(super::bing::BingEngine {
                region,
                nav_timeout: timeout,
            }),
            (ConnectionMode::UserChrome, _) => Box::new(super::google::GoogleEngine {
                region,
                nav_timeout: timeout,
            }),
            (ConnectionMode::Headless, _) => Box::new(super::bing::BingEngine {
                region,
                nav_timeout: timeout,
            }),
        },
    }
}

pub struct RateLimiter {
    last_request: Mutex<Instant>,
    min_interval_ms: u64,
    penalty_ms: Mutex<u64>,
}

impl RateLimiter {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            last_request: Mutex::new(Instant::now() - std::time::Duration::from_secs(10)),
            min_interval_ms,
            penalty_ms: Mutex::new(0),
        }
    }

    pub async fn wait(&self) {
        use rand::Rng;
        let mut last = self.last_request.lock().await;
        let penalty = *self.penalty_ms.lock().await;
        let elapsed = last.elapsed().as_millis() as u64;
        let jitter = rand::rng().random_range(0..3000u64);
        let required = self.min_interval_ms + jitter + penalty;
        if elapsed < required {
            tokio::time::sleep(std::time::Duration::from_millis(required - elapsed)).await;
        }
        *last = Instant::now();
    }

    pub async fn backoff(&self) {
        let mut penalty = self.penalty_ms.lock().await;
        *penalty = (*penalty + 5000).min(30_000);
        tracing::info!(new_penalty_ms = *penalty, "Rate limiter backoff increased");
    }

    pub async fn reset_penalty(&self) {
        let mut penalty = self.penalty_ms.lock().await;
        if *penalty > 0 {
            tracing::debug!(was = *penalty, "Rate limiter penalty reset");
            *penalty = 0;
        }
    }
}

pub fn format_search_results(query: &str, engine: &str, results: &[SearchResult]) -> String {
    let mut out = format!(
        "Found {} results for \"{}\" (via {}):\n\n",
        results.len(), query, engine
    );
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!(
            "{}. [{}]({})\n   {}\n\n",
            i + 1, r.title, r.url, r.snippet
        ));
    }
    out
}

pub fn to_mcp_error<E: std::fmt::Display>(e: E) -> rmcp::model::ErrorData {
    rmcp::model::ErrorData::internal_error(format!("{e}"), None)
}

pub async fn search_with_fallback(
    primary: Box<dyn SearchEngine>,
    fallback_mode: &ConnectionMode,
    page: &Page,
    query: &str,
    count: usize,
    region: &'static str,
) -> anyhow::Result<(String, Vec<SearchResult>)> {
    let primary_name = primary.name().to_string();
    let mut last_error;

    match primary.search(page, query, count).await {
        Ok(results) if !results.is_empty() => {
            return Ok((primary_name, results));
        }
        Ok(_empty) => {
            tracing::warn!(engine = %primary_name, "Empty results on first attempt");
            last_error = "empty results".to_string();
        }
        Err(e) => {
            tracing::warn!(engine = %primary_name, error = %e, "First attempt failed");
            last_error = e.to_string();
        }
    }

    let retry_backoff = if last_error.to_lowercase().contains("captcha") {
        std::time::Duration::from_secs(8)
    } else {
        std::time::Duration::from_secs(3)
    };
    tracing::info!(engine = %primary_name, backoff_secs = retry_backoff.as_secs(), "Retrying");
    tokio::time::sleep(retry_backoff).await;

    match primary.search(page, query, count).await {
        Ok(results) if !results.is_empty() => {
            return Ok((primary_name, results));
        }
        Ok(_empty) => {
            tracing::warn!(engine = %primary_name, "Retry also returned empty");
        }
        Err(e) => {
            tracing::warn!(engine = %primary_name, error = %e, "Retry also failed");
            last_error = e.to_string();
        }
    }

    let fb_timeout = match fallback_mode {
        ConnectionMode::Headless => 25,
        ConnectionMode::UserChrome => 15,
    };
    let fallback: Option<Box<dyn SearchEngine>> = match (primary_name.as_str(), fallback_mode) {
        ("google", _) => Some(Box::new(super::bing::BingEngine { region, nav_timeout: fb_timeout })),
        ("duckduckgo", _) => Some(Box::new(super::bing::BingEngine { region, nav_timeout: fb_timeout })),
        ("bing", _) => Some(Box::new(super::google::GoogleEngine {
            region,
            nav_timeout: fb_timeout,
        })),
        _ => None,
    };

    if let Some(fb) = fallback {
        tracing::info!(fallback = fb.name(), "Trying fallback engine");
        match fb.search(page, query, count).await {
            Ok(results) if !results.is_empty() => {
                return Ok((fb.name().to_string(), results));
            }
            Ok(_) => tracing::warn!(engine = fb.name(), "Fallback also empty"),
            Err(e) => {
                tracing::warn!(engine = fb.name(), error = %e, "Fallback failed");
                last_error = e.to_string();
            }
        }
    }

    anyhow::bail!(
        "Search failed for \"{}\". Last error: {}. Try --remote-url with your Chrome.",
        query, last_error
    )
}

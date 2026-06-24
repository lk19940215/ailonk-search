use moka::future::Cache;
use std::time::Duration;

#[derive(Clone)]
pub struct ContentCache {
    cache: Cache<String, CachedContent>,
    enabled: bool,
}

#[derive(Clone, Debug)]
pub struct CachedContent {
    pub title: String,
    pub content: String,
}

impl ContentCache {
    pub fn new(ttl_secs: u64) -> Self {
        let enabled = ttl_secs > 0;
        let cache = if enabled {
            Cache::builder()
                .max_capacity(100)
                .time_to_live(Duration::from_secs(ttl_secs))
                .build()
        } else {
            Cache::builder().max_capacity(1).build()
        };
        Self { cache, enabled }
    }

    /// Generate cache key from URL + parameters
    pub fn key(url: &str, include_links: bool, max_length: usize) -> String {
        format!("{}:{}:{}", url, include_links, max_length)
    }

    pub async fn get(&self, key: &str) -> Option<CachedContent> {
        if !self.enabled {
            return None;
        }
        self.cache.get(key).await
    }

    pub async fn insert(&self, key: String, content: CachedContent) {
        if !self.enabled {
            return;
        }
        self.cache.insert(key, content).await;
    }
}

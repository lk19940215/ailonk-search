use clap::{Parser, Subcommand};

#[derive(Parser, Debug, Clone)]
#[command(name = "ailonk-search")]
#[command(about = "Chrome CDP-based MCP Server for web search")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    #[command(flatten)]
    pub args: Args,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Run as MCP Server (default, used by AI tools)
    Serve,
    /// Setup Chrome for optimal experience (one-time)
    Setup,
    /// Sync login state (cookies, sessions) from main Chrome to debug profile
    Sync,
    /// Remove debug profile symlink and print recovery instructions
    Cleanup,
    /// Run all test scenarios and output a structured report
    TestAll,
    /// Dev: test web search directly (no MCP)
    TestSearch {
        query: String,
        #[arg(long, default_value = "auto")]
        engine: String,
        #[arg(long, default_value = "10")]
        count: usize,
    },
    /// Dev: test page reading directly (no MCP)
    TestRead {
        url: String,
        #[arg(long, default_value = "15000")]
        max_length: usize,
    },
    /// Dev: test search + read flow (simulates MCP search_and_read)
    TestSearchAndRead {
        query: String,
        #[arg(long, default_value = "3")]
        read_count: usize,
        #[arg(long, default_value = "5000")]
        max_length: usize,
    },
}

#[derive(Parser, Debug, Clone)]
pub struct Args {
    /// Connect to existing Chrome at this URL
    #[arg(long, global = true)]
    pub remote_url: Option<String>,

    /// Force headless Chrome (skip auto-connect)
    #[arg(long, global = true)]
    pub headless: bool,

    /// Path to Chrome executable
    #[arg(long, global = true)]
    pub chrome_path: Option<String>,

    /// Default search engine: auto|google|bing|duckduckgo
    #[arg(long, default_value = "auto", global = true)]
    pub engine: String,

    /// Region for search: auto (detect from system locale), cn (China mainland), global
    #[arg(long, default_value = "auto", global = true)]
    pub region: String,

    /// Maximum concurrent tabs
    #[arg(long, default_value = "5", value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..), global = true)]
    pub max_tabs: usize,

    /// Pass-through Chrome launch arguments (comma-separated)
    #[arg(long, global = true)]
    pub chrome_args: Option<String>,

    /// Cache TTL in seconds (0 to disable, default: disabled)
    #[arg(long, default_value = "0", global = true)]
    pub cache_ttl: u64,

    /// Allow accessing private/internal URLs (127.0.0.1, 192.168.*, etc.)
    #[arg(long, global = true)]
    pub allow_private_urls: bool,
}

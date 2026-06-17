# Chrome Search MCP — P0 MVP 实施计划

> 参考文档：[ARCHITECTURE.md](./ARCHITECTURE.md)
>
> 本文档定义 P0 MVP 从初始化到集成测试的完整实施路径。
> 每个任务包含目标、架构关联、实施细节、验收标准。

---

## 0. 代码规范

### 0.1 项目约定

| 约定 | 规则 |
|------|------|
| Rust Edition | 2021 |
| MSRV | 1.85+ (chromiumoxide 0.9 要求) |
| 异步运行时 | tokio (features = ["full"]) |
| 错误处理 | `anyhow::Result` 用于应用层；MCP handler 返回 `McpError` |
| 日志 | `tracing` 宏 (`info!`, `warn!`, `error!`, `debug!`) |
| 命名 | 模块: snake_case; 结构体: PascalCase; 常量: UPPER_SNAKE_CASE |
| 注释 | 仅在非显而易见的逻辑处添加；不写显而易见的注释 |

### 0.2 错误处理模式

```rust
// 应用层: anyhow
pub async fn connect_chrome(args: &Args) -> anyhow::Result<BrowserManager> { ... }

// MCP handler: 使用 to_mcp_error helper 转换
#[tool(description = "...")]
async fn web_search(&self, Parameters(params): Parameters<WebSearchParams>)
    -> Result<CallToolResult, McpError>
{
    let results = self.do_search(params).await.map_err(to_mcp_error)?;
    Ok(CallToolResult::success(vec![Content::text(format_results(&results))]))
}

// to_mcp_error 定义在 search/engine.rs (见 T4.1)
fn to_mcp_error(e: anyhow::Error) -> rmcp::model::ErrorData {
    rmcp::model::ErrorData::internal_error(format!("{:#}", e), None::<()>)
}
```

### 0.3 MCP 返回格式约定

- 成功: `CallToolResult::success(vec![Content::text(...)])`
- 失败: `CallToolResult` with `is_error: true` + 人类可读错误信息
- 截图: `Content::image(base64, mime_type)` 或文件路径
- 所有文本输出使用 Markdown 格式

---

## 1. 任务总览

```
T1 项目初始化
 └─→ T2 BrowserManager (Chrome CDP 连接)
      └─→ T3 MCP Server 骨架 (rmcp stdio)
           ├─→ T4 web_search (Google + Bing)
           └─→ T5 read_page (dom_smoothie)
                └─→ T6 集成测试
```

| 任务 | 预估 | 依赖 | 产物 |
|------|------|------|------|
| T1 | - | 无 | 可编译项目骨架 |
| T2 | - | T1 | BrowserManager: 可连接 Chrome |
| T3 | - | T1 | MCP Server: 可被 AI 工具发现 |
| T4 | - | T2 + T3 | web_search: 返回搜索结果 |
| T5 | - | T2 + T3 | read_page: 返回 Markdown 正文 |
| T6 | - | T4 + T5 | 端到端验证通过 |

---

## 0.4 测试策略

**核心原则：不需要打包+配置 MCP 来调试。** 日常开发使用三板斧：

| 方案 | 反馈速度 | 用途 | 需要 Chrome |
|------|---------|------|------------|
| **1. 单元测试** (fixture HTML) | ⚡ 毫秒级 | 解析/提取/格式化逻辑 | ❌ |
| **2. CLI 子命令** (`test-search` / `test-read`) | ⚡ 秒级 | 真实 Chrome 行为验证 | ✅ |
| **3. MCP Inspector** (`--cli` 模式) | 秒级 | 完整 MCP 协议调试 | ✅ |

### 架构：核心函数分层（测试复用）

```rust
// 核心逻辑独立于 MCP 层，所有测试路径共享
pub mod core {
    pub async fn web_search(bm: &BrowserManager, params: WebSearchParams) -> anyhow::Result<String> { ... }
    pub async fn read_page(bm: &BrowserManager, params: ReadPageParams) -> anyhow::Result<String> { ... }
}

// MCP handler — 薄包装 core::
async fn web_search(&self, Parameters(p): ...) -> Result<CallToolResult, McpError> {
    let text = core::web_search(&self.browser_manager, p).await.map_err(to_mcp_error)?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

// CLI 子命令 — 直接调 core::
// 单元测试 — 测 parse/extract 纯函数
// handler 测试 — 调 MCP 方法或 core::
```

### 方案 1: 单元测试

- 保存 Google/Bing 搜索结果 HTML 为 fixture 文件 (`tests/fixtures/`)
- 把 JS evaluate 的解析逻辑抽成纯 Rust 函数 `parse_google_html(html) -> Vec<SearchResult>`
- dom_smoothie 提取测试：输入静态 HTML，验证输出不含导航/广告
- 需要真实 Chrome 的测试标记 `#[ignore]`

```rust
#[test]
fn extract_article_content() {
    let html = include_str!("fixtures/rust_book_ownership.html");
    let result = ContentExtractor::extract(html, "https://...", 5000).unwrap();
    assert!(result.content.contains("ownership"));
    assert!(!result.content.contains("nav"));
}
```

### 方案 2: CLI 子命令

给二进制加 `test-search` / `test-read` 子命令，**不经过 MCP 协议**：

```bash
# 搜索测试
cargo run -- test-search "tokio async runtime" --engine auto

# 阅读测试
cargo run -- test-read "https://doc.rust-lang.org/book/ch01-01-installation.html"
```

CLI 定义：
```rust
#[derive(Subcommand)]
pub enum Commands {
    /// MCP Server 模式 (默认，给 AI 工具用)
    Serve,
    /// 开发调试：搜索
    TestSearch { query: String, #[arg(long, default_value = "auto")] engine: String },
    /// 开发调试：阅读
    TestRead { url: String, #[arg(long, default_value = "15000")] max_length: usize },
}
```

### 方案 3: MCP Inspector

**无需配置 Cursor/Claude Code**，直接调试 MCP 协议：

```bash
# 编译
cargo build

# UI 模式 (浏览器)
npx @modelcontextprotocol/inspector target/debug/ailonk-search -- --headless

# CLI 模式 (快速反馈)
npx @modelcontextprotocol/inspector --cli target/debug/ailonk-search -- --headless \
  --method tools/call --tool-name web_search --tool-arg query="tokio tutorial"
```

> **注意：** 日志必须写 stderr（`tracing` 默认 stderr，无需额外配置），stdout 专属 MCP JSON-RPC。

---

## T1: 项目初始化

> 关联：ARCHITECTURE.md §7.2 核心依赖, §7.3 项目结构

### 目标

创建 Cargo 项目，配置所有 P0 依赖，搭建目录结构和 CLI 入口。

### 实施

**T1.1: 项目创建与依赖**

```bash
cd /Users/longkuo/Desktop/AI/ailonk-search
cargo init --name ailonk-search
```

Cargo.toml:
```toml
[package]
name = "ailonk-search"
version = "0.1.0"
edition = "2021"
rust-version = "1.85"
description = "Chrome CDP-based MCP Server for web search and content extraction"

[dependencies]
# --- MCP Server ---
rmcp = { version = "1.7", features = ["server", "macros", "schemars"] }

# --- Chrome CDP ---
chromiumoxide = { version = "0.9", features = ["fetcher", "rustls", "zip8"] }
futures = "0.3"

# --- Async ---
tokio = { version = "1", features = ["full"] }

# --- HTML 处理 ---
dom_smoothie = { version = "0.18", features = ["serde"] }
scraper = "0.27"              # P2 备用 (DDG html 解析), P0 用 JS evaluate

# --- 序列化 ---
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "1"

# --- CLI ---
clap = { version = "4", features = ["derive"] }

# --- 日志 ---
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# --- 错误处理 ---
anyhow = "1"

# --- 搜索引擎 ---
async-trait = "0.1"           # SearchEngine trait
urlencoding = "2"             # query URL 编码
rand = "0.9"                  # 请求限速 jitter (真随机)

# --- P1 预留 (暂不启用) ---
# moka = { version = "0.12", features = ["future"] }
# htmd = "0.5"
```

**T1.2: 目录结构**

```
src/
├── main.rs
├── cli.rs              # CLI 参数定义
├── server/
│   ├── mod.rs
│   └── tools.rs        # MCP Tool 定义
├── browser/
│   ├── mod.rs
│   └── manager.rs      # Chrome 生命周期
├── search/
│   ├── mod.rs
│   ├── engine.rs       # SearchEngine trait
│   ├── google.rs       # Google 解析
│   └── bing.rs         # Bing 解析
└── extract/
    ├── mod.rs
    └── content.rs      # dom_smoothie 封装
```

**T1.3: CLI 参数 (cli.rs)**

```rust
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

    /// Default search engine: auto|google|bing
    #[arg(long, default_value = "auto", global = true)]
    pub engine: String,

    /// Maximum concurrent tabs
    #[arg(long, default_value = "5", global = true)]
    pub max_tabs: usize,

    /// Pass-through Chrome launch arguments (comma-separated)
    #[arg(long, global = true)]
    pub chrome_args: Option<String>,
}
```

**T1.4: main.rs 入口骨架**

```rust
mod cli;
mod server;
mod browser;
mod search;
mod extract;

use clap::Parser;
use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("ailonk_search=info")
        .with_writer(std::io::stderr)  // MCP: stdout 专属 JSON-RPC
        .init();

    let cli = Cli::parse();
    tracing::info!("Starting ailonk-search");

    match cli.command {
        None | Some(Commands::Serve) => {
            // MCP Server 模式 (默认)
            // T2 + T3 实现
            todo!("MCP server")
        }
        Some(Commands::TestSearch { query, engine, count }) => {
            // CLI 测试：搜索
            // T4 实现后接入 core::web_search
            todo!("test-search")
        }
        Some(Commands::TestRead { url, max_length }) => {
            // CLI 测试：阅读
            // T5 实现后接入 core::read_page
            todo!("test-read")
        }
    }

    Ok(())
}
```

### 验收标准

- [ ] `cargo build` 成功，无 warning
- [ ] `cargo run -- --help` 输出 CLI 帮助
- [ ] 所有 mod.rs 文件存在且可编译
- [ ] `cargo clippy` 无 error

---

## T2: BrowserManager — Chrome CDP 连接

> 关联：ARCHITECTURE.md §3.2 Chrome 连接策略, §3.3 BrowserManager

### 目标

实现 Chrome 浏览器连接管理：自动检测/连接用户 Chrome 或启动 headless 实例。

### 核心数据结构

```rust
// browser/manager.rs
use std::sync::Arc;
use chromiumoxide::{Browser, BrowserConfig};
use chromiumoxide::handler::Handler;
use futures::StreamExt; // handler.next() 需要此 import
use tokio::task::JoinHandle;

pub enum ConnectionMode {
    UserChrome,   // 复用用户 Chrome (→ 默认 Google)
    Headless,     // 自启 headless (→ 默认 Bing)
}

pub struct BrowserManager {
    browser: Arc<Browser>,
    _handler_handle: JoinHandle<()>,  // 前缀 _ 表示只需保持存活
    mode: ConnectionMode,
}
```

### 实施

**T2.1: Chrome 连接逻辑**

按 ARCHITECTURE.md §3.2 的三种模式实现：

```rust
impl BrowserManager {
    pub async fn new(args: &Args) -> anyhow::Result<Self> {
        if let Some(ref url) = args.remote_url {
            Self::connect_remote(url).await
        } else if args.headless {
            Self::launch_headless(args).await
        } else {
            // auto-connect: 尝试 9222 → fallback headless
            match Self::connect_remote("http://127.0.0.1:9222").await {
                Ok(mgr) => Ok(mgr),
                Err(_) => {
                    tracing::info!("No Chrome found on :9222, launching headless");
                    Self::launch_headless(args).await
                }
            }
        }
    }

    async fn connect_remote(url: &str) -> anyhow::Result<Self> {
        // 从 /json/version 获取 WebSocket URL
        let (browser, handler) = Browser::connect(url).await?;
        tracing::info!("Connected to existing Chrome at {}", url);
        Ok(Self::spawn_handler(browser, handler, ConnectionMode::UserChrome))
    }

    async fn launch_headless(args: &Args) -> anyhow::Result<Self> {
        let mut builder = BrowserConfig::builder()
            .no_sandbox()            // Docker 兼容
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--disable-blink-features=AutomationControlled") // 隐藏自动化特征
            .env("LANG", "en_US.UTF-8"); // chromiumoxide 要求英文环境

        if let Some(ref path) = args.chrome_path {
            builder = builder.chrome_executable(path);
        }

        if let Some(ref extra_args) = args.chrome_args {
            for arg in extra_args.split(',') {
                builder = builder.arg(arg.trim());
            }
        }

        let config = builder.build()
            .map_err(|e| anyhow::anyhow!("Chrome config error: {}", e))?;

        let (browser, handler) = Browser::launch(config).await?;
        tracing::info!("Launched headless Chrome");
        Ok(Self::spawn_handler(browser, handler, ConnectionMode::Headless))
    }
}
```

**T2.2: Handler 事件循环**

chromiumoxide 必须持续驱动 handler，否则 CDP 操作会挂起。需要 `futures::StreamExt`：

```rust
impl BrowserManager {
    fn spawn_handler(browser: Browser, mut handler: Handler, mode: ConnectionMode) -> Self {
        let browser = Arc::new(browser);
        let handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(e) = event {
                    tracing::warn!("CDP handler error: {:?}", e);
                    break;
                }
            }
            tracing::debug!("CDP handler loop ended");
        });
        Self { browser, _handler_handle: handle, mode }
    }
}
```

**T2.3: Chrome 路径检测**

不使用 `which` crate，用标准库检测：

```rust
fn find_chrome_path() -> Option<String> {
    let candidates = if cfg!(target_os = "macos") {
        vec!["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
    } else if cfg!(target_os = "linux") {
        vec!["chromium-browser", "chromium", "google-chrome", "google-chrome-stable"]
    } else if cfg!(target_os = "windows") {
        vec![r"C:\Program Files\Google\Chrome\Application\chrome.exe"]
    } else {
        vec![]
    };

    for path in candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.into());
        }
    }
    None
}
```

> **注意：** `chromiumoxide` 的 `fetcher` feature 可自动下载 Chrome。如果 `find_chrome_path()` 返回 None，`Browser::launch` 会尝试自动下载。

**T2.4: Headless Stealth — webdriver 检测绕过**

Headless Chrome 默认暴露 `navigator.webdriver = true`，这是搜索引擎检测自动化的最基础手段。需在每个新 Tab 创建时注入覆盖脚本：

```rust
impl BrowserManager {
    /// 对 headless 模式的页面注入 stealth 脚本，覆盖 navigator.webdriver
    pub async fn stealth_page(&self, page: &Page) -> anyhow::Result<()> {
        if matches!(self.mode, ConnectionMode::Headless) {
            page.execute(
                chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams::new(
                    "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})"
                )
            ).await?;
        }
        Ok(())
    }
}
```

> **注意：** `--disable-blink-features=AutomationControlled`（T2.1 已添加）移除 Chrome 层面的自动化标记，`stealth_page()` 在 JS 层面覆盖 `navigator.webdriver`，两者互补。

**T2.5: 公共方法**

```rust
impl BrowserManager {
    pub fn browser(&self) -> &Arc<Browser> { &self.browser }
    pub fn mode(&self) -> &ConnectionMode { &self.mode }
}

// 退出时: BrowserManager drop → _handler_handle drop → handler abort
// Arc<Browser> 最后一个引用释放时 → Chrome 进程自动关闭 (由 chromiumoxide 管理)
// 不需要显式 shutdown: Browser::close() 需要 &mut self，与 Arc 不兼容
// chromiumoxide 的 Browser Drop impl 会发送 Browser.close CDP 命令
```

### 验收标准

- [ ] `--remote-url http://127.0.0.1:9222` 可连接已运行的 Chrome（需先手动开启远程调试）
- [ ] `--headless` 可自动启动 headless Chrome
- [ ] auto-connect 模式: 有 Chrome → 连接成功；无 Chrome → fallback headless
- [ ] 退出时 Chrome 进程正常关闭（headless 模式）
- [ ] 连接失败时返回清晰的错误信息
- [ ] `tracing::info!` 输出连接模式和 Chrome 版本

---

## T3: MCP Server 骨架

> 关联：ARCHITECTURE.md §3.1 Layer 1, §7.2 rmcp 依赖

### 目标

基于 rmcp 实现 stdio MCP Server，注册 `web_search` 和 `read_page` 工具 schema，工具暂返回 placeholder。

### 核心数据结构

```rust
// server/mod.rs
use std::sync::Arc;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::*;
use rmcp::schemars;
use crate::browser::BrowserManager;

#[derive(Clone)]  // rmcp serve() 要求 handler 可 Clone
pub struct SearchServer {
    browser_manager: Arc<BrowserManager>,
    default_engine: String,
}

impl SearchServer {
    pub fn new(browser_manager: Arc<BrowserManager>, default_engine: String) -> Self {
        Self { browser_manager, default_engine }
    }
}
```

### 实施

**T3.1: Tool 参数定义 (server/tools.rs)**

```rust
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    pub query: String,
    #[serde(default)]
    pub engine: EngineChoice,
    #[serde(default = "default_count")]
    pub count: usize,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EngineChoice {
    #[default]
    Auto,
    Google,
    Bing,
    Duckduckgo,
}

fn default_count() -> usize { 10 }

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPageParams {
    pub url: String,
    // P0: include_links 暂不实现，dom_smoothie Markdown 输出默认保留链接
    // P1 可通过后处理 strip 链接
    #[serde(default = "default_true")]
    pub include_links: bool,
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

fn default_true() -> bool { true }
fn default_max_length() -> usize { 15000 }
```

**T3.2: Server 实现**

```rust
use rmcp::{ServerHandler, ServiceExt};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::tool;

#[tool_router]
impl SearchServer {
    #[tool(description = "Search the web using a real Chrome browser. Returns multiple results with titles, URLs, and snippets in a single call. Engine 'auto' selects Google for user Chrome or Bing for headless.")]
    async fn web_search(
        &self,
        Parameters(params): Parameters<WebSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        // T4 实现
        Ok(CallToolResult::success(vec![Content::text("web_search placeholder")]))
    }

    #[tool(description = "Fetch a URL using Chrome and extract the main content as clean Markdown. Strips navigation, ads, scripts, and styles.")]
    async fn read_page(
        &self,
        Parameters(params): Parameters<ReadPageParams>,
    ) -> Result<CallToolResult, McpError> {
        // T5 实现
        Ok(CallToolResult::success(vec![Content::text("read_page placeholder")]))
    }
}

#[tool_handler]
impl ServerHandler for SearchServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "ailonk-search".into(),
                version: env!("CARGO_PKG_VERSION").into(),
            },
            instructions: Some(
                "Web search and content extraction via Chrome CDP. \
                 Tools: web_search (search the web), read_page (extract page content as Markdown)."
                .into()
            ),
        }
    }
}
```

**T3.3: main.rs 完整启动流程**

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("ailonk_search=info")
        .init();

    let args = Args::parse();

    // 1. 连接 Chrome
    let browser_manager = BrowserManager::new(&args).await?;
    let browser_manager = Arc::new(browser_manager);

    // 2. 创建 MCP Server
    let server = SearchServer::new(browser_manager.clone(), args.engine.clone());

    // 3. 启动 stdio 传输
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let running_server = server.serve(transport).await?;

    // 4. 等待退出
    running_server.waiting().await?;

    // 5. 清理
    // browser_manager 引用计数归零后自动清理
    // 或显式调用 shutdown

    Ok(())
}
```

### 验收标准

- [ ] `cargo run` 启动后不崩溃，等待 stdio 输入
- [ ] MCP Inspector (`npx @anthropic-ai/mcp-inspector`) 连接后可看到 2 个工具
- [ ] 工具的 JSON Schema 与 ARCHITECTURE.md §5.1/§5.2 定义一致
- [ ] 调用 `web_search` / `read_page` 返回 placeholder 文本（不崩溃）
- [ ] Server info 中 name = "ailonk-search"，version = Cargo.toml 版本

---

## T4: web_search — 搜索引擎实现

> 关联：ARCHITECTURE.md §4 搜索引擎策略, §3.5 内容提取管线 (第一层), §5.1 web_search Tool

### 目标

实现 `web_search` 工具：导航到搜索引擎，提取自然搜索结果，排除广告，返回格式化文本。

### 核心数据结构

```rust
// search/engine.rs
use serde::Deserialize;

#[derive(Debug, Deserialize)]  // Deserialize: page.evaluate(js).into_value() 反序列化需要
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[async_trait::async_trait]
pub trait SearchEngine: Send + Sync {
    fn name(&self) -> &str;
    async fn search(
        &self,
        page: &chromiumoxide::Page,
        query: &str,
        count: usize,
    ) -> anyhow::Result<Vec<SearchResult>>;
}

/// 引擎选择：接受 EngineChoice enum 而非 &str，与 WebSearchParams 类型匹配
pub fn select_engine(choice: &EngineChoice, connection_mode: &ConnectionMode) -> Box<dyn SearchEngine> {
    match choice {
        EngineChoice::Google => Box::new(GoogleEngine),
        EngineChoice::Bing => Box::new(BingEngine),
        EngineChoice::Auto => match connection_mode {
            ConnectionMode::UserChrome => Box::new(GoogleEngine),
            ConnectionMode::Headless => Box::new(BingEngine),
        },
        _ => Box::new(BingEngine),
    }
}

/// anyhow::Error → rmcp ErrorData 转换 helper
pub fn to_mcp_error(e: anyhow::Error) -> rmcp::model::ErrorData {
    rmcp::model::ErrorData::internal_error(format!("{:#}", e), None::<()>)
}
```

### 实施

**T4.1: SearchEngine trait + 引擎选择 (search/engine.rs)**

`select_engine` 已在核心数据结构中定义（接受 `&EngineChoice` enum），此处不再重复。

**T4.2: Google 搜索 (search/google.rs)**

参考 ARCHITECTURE.md §4.5 搜索结果解析策略：

```rust
pub struct GoogleEngine;

#[async_trait::async_trait]
impl SearchEngine for GoogleEngine {
    fn name(&self) -> &str { "google" }

    async fn search(&self, page: &Page, query: &str, count: usize) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "https://www.google.com/search?q={}&num={}&hl=en",
            urlencoding::encode(query),
            count.min(20)
        );

        page.goto(&url).await?;

        // Cookie 同意页处理 (GDPR)
        handle_consent_page(page, "google").await?;

        // 等待搜索结果渲染
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // CAPTCHA 检测
        let title = page.get_title().await?.unwrap_or_default();
        let current_url = page.url().await?.unwrap_or_default();
        if detect_captcha(&title, &current_url) {
            anyhow::bail!("Google CAPTCHA detected. Try --remote-url with your Chrome.");
        }

        // JS 提取自然搜索结果 (排除广告)
        let js = r#"
            Array.from(document.querySelectorAll('div.g:not([data-text-ad])'))
                .filter(el => el.querySelector('h3') && el.querySelector('a[href]'))
                .map(el => ({
                    title: el.querySelector('h3')?.textContent?.trim() || '',
                    url: el.querySelector('a[href]')?.href || '',
                    snippet: (el.querySelector('[data-sncf]') || el.querySelector('.VwiC3b') || el.querySelector('span.st'))?.textContent?.trim() || ''
                }))
                .filter(r => r.title && r.url && !r.url.startsWith('https://www.google.com'))
        "#;

        let results: Vec<SearchResult> = page.evaluate(js).await?.into_value()?;
        Ok(results.into_iter().take(count).collect())
    }
}
```

**T4.3: Bing 搜索 (search/bing.rs)**

```rust
pub struct BingEngine;

#[async_trait::async_trait]
impl SearchEngine for BingEngine {
    fn name(&self) -> &str { "bing" }

    async fn search(&self, page: &Page, query: &str, count: usize) -> anyhow::Result<Vec<SearchResult>> {
        let url = format!(
            "https://www.bing.com/search?q={}&count={}",
            urlencoding::encode(query),
            count.min(20)
        );

        page.goto(&url).await?;

        // Cookie 同意横幅处理
        handle_consent_page(page, "bing").await?;

        // 等待搜索结果渲染
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let title = page.get_title().await?.unwrap_or_default();
        if title.contains("unusual traffic") || title.contains("robot") {
            anyhow::bail!("Bing CAPTCHA detected");
        }

        let js = r#"
            Array.from(document.querySelectorAll('li.b_algo'))
                .map(el => ({
                    title: el.querySelector('h2 a')?.textContent?.trim() || '',
                    url: el.querySelector('h2 a')?.href || '',
                    snippet: el.querySelector('p')?.textContent?.trim() || ''
                }))
                .filter(r => r.title && r.url)
        "#;

        let results: Vec<SearchResult> = page.evaluate(js).await?.into_value()?;
        Ok(results.into_iter().take(count).collect())
    }
}
```

**T4.4: CAPTCHA 检测公共函数**

```rust
// search/engine.rs
pub fn detect_captcha(title: &str, url: &str) -> bool {
    let title_lower = title.to_lowercase();
    let url_lower = url.to_lowercase();
    title_lower.contains("unusual traffic")
        || title_lower.contains("are not a robot")
        || title_lower.contains("captcha")
        || url_lower.contains("/sorry/")
        || url_lower.contains("captcha")
        || url_lower.contains("challenge")
}
```

**T4.5: 请求限速 — 全局限速器 + jitter**

参考 ARCHITECTURE.md §4.6 MVP 反爬措施。AI Agent 使用模式为突发式（一个任务内 3-10 次搜索），
需要跨请求的全局限速器，而非仅单次请求内的延迟：

```rust
use rand::Rng;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Instant;

pub struct RateLimiter {
    last_request: Mutex<Instant>,
    min_interval_ms: u64,  // 最小请求间隔
}

impl RateLimiter {
    pub fn new(min_interval_ms: u64) -> Self {
        Self {
            last_request: Mutex::new(Instant::now() - std::time::Duration::from_secs(10)),
            min_interval_ms,
        }
    }

    pub async fn wait(&self) {
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed().as_millis() as u64;
        let jitter = rand::rng().random_range(0..3000u64);
        let required = self.min_interval_ms + jitter;
        if elapsed < required {
            tokio::time::sleep(std::time::Duration::from_millis(required - elapsed)).await;
        }
        *last = Instant::now();
    }
}

// 在 SearchServer 中持有全局限速器实例
// min_interval_ms = 2000 → 实际间隔 2-5s (含 jitter)
```

**T4.6: Cookie 同意页处理**

Google/Bing 首次访问可能弹出 GDPR Cookie 同意页面，阻断搜索流程：

```rust
async fn handle_consent_page(page: &Page, engine_name: &str) -> anyhow::Result<()> {
    let current_url = page.url().await?.unwrap_or_default();

    if current_url.contains("consent.google.com") {
        // Google 同意页：点击 "Accept all" 按钮
        let js = r#"
            const btn = document.querySelector('button[aria-label*="Accept"]')
                || document.querySelector('form[action*="consent"] button');
            if (btn) btn.click();
        "#;
        page.evaluate(js).await.ok();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    if engine_name == "bing" {
        // Bing Cookie 横幅：尝试点击接受按钮
        let js = r#"
            const btn = document.querySelector('#bnp_btn_accept')
                || document.querySelector('.bnp_btn_accept');
            if (btn) btn.click();
        "#;
        page.evaluate(js).await.ok();
    }

    Ok(())
}
```

**T4.7: web_search handler 集成**

将 placeholder 替换为真实实现：

```rust
#[tool(description = "Search the web...")]
async fn web_search(&self, Parameters(params): Parameters<WebSearchParams>) -> Result<CallToolResult, McpError> {
    // 全局限速器等待
    self.rate_limiter.wait().await;

    let engine = select_engine(&params.engine, self.browser_manager.mode());

    // 创建新 Tab (P0 串行，不用 Tab 池)
    let page = self.browser_manager.browser()
        .new_page("about:blank").await
        .map_err(to_mcp_error)?;

    // Headless stealth: 覆盖 navigator.webdriver
    self.browser_manager.stealth_page(&page).await.map_err(to_mcp_error)?;

    let results = engine.search(&page, &params.query, params.count).await
        .map_err(to_mcp_error)?;

    page.close().await.ok(); // 搜索完关闭 Tab

    if results.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            format!("No results found for \"{}\". The search engine may have blocked the request.", params.query)
        )]));
    }

    let text = format_search_results(&params.query, engine.name(), &results);
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn format_search_results(query: &str, engine: &str, results: &[SearchResult]) -> String {
    let mut out = format!("Found {} results for \"{}\" (via {}):\n\n", results.len(), query, engine);
    for (i, r) in results.iter().enumerate() {
        out.push_str(&format!(
            "{}. [{}]({})\n   {}\n\n",
            i + 1, r.title, r.url, r.snippet
        ));
    }
    out
}
```

### 验收标准

- [ ] `web_search(query="rust async tutorial", engine="auto")` 返回 ≥5 条结果 (受网络/地区影响)
- [ ] 返回格式为 Markdown：`序号. [标题](URL)\n   摘要`
- [ ] 复用用户 Chrome 时 Google 搜索正常（auto → Google）
- [ ] Headless 模式 Bing 搜索正常（auto → Bing）
- [ ] `engine="google"` / `engine="bing"` 手动指定有效
- [ ] 返回的 URL 不含 Google/Bing 的 redirect 包装（清洗过的直链）
- [ ] 返回结果中不含广告（无 `[data-text-ad]` / `.b_ad` 内容）
- [ ] CAPTCHA 页面被检测到并返回清晰的 MCP 错误信息
- [ ] 每次搜索有 2-5 秒的随机延迟

---

## T5: read_page — 正文提取实现

> 关联：ARCHITECTURE.md §3.5 内容提取管线 (第二层), §5.2 read_page Tool

### 目标

实现 `read_page` 工具：Chrome 渲染页面，dom_smoothie 提取正文，返回 Markdown。

### 实施

**T5.1: dom_smoothie 正文提取封装 (extract/content.rs)**

```rust
use dom_smoothie::{Readability, Config, TextMode};

pub struct ContentExtractor;

impl ContentExtractor {
    pub fn extract(html: &str, url: &str, max_length: usize) -> anyhow::Result<ExtractedContent> {
        let config = Config {
            text_mode: TextMode::Markdown,
            ..Default::default()
        };

        let mut readability = Readability::new(html, Some(url), Some(config))?;
        let article = readability.parse()?;

        let title = article.title.to_string();          // title 是 String, 非 Option
        let mut content = article.text_content.to_string(); // text_content 是 StrTendril, 需 .to_string()

        if content.len() > max_length {
            // 在最后一个完整段落处截断
            if let Some(pos) = content[..max_length].rfind("\n\n") {
                content.truncate(pos);
            } else {
                content.truncate(max_length);
            }
            content.push_str("\n\n(... content truncated)");
        }

        Ok(ExtractedContent { title, content })
    }
}

pub struct ExtractedContent {
    pub title: String,
    pub content: String,
}
```

**T5.2: read_page handler 实现**

```rust
#[tool(description = "Fetch a URL using Chrome and extract the main content as clean Markdown...")]
async fn read_page(&self, Parameters(params): Parameters<ReadPageParams>) -> Result<CallToolResult, McpError> {
    // 创建 Tab 并导航
    let page = self.browser_manager.browser()
        .new_page("about:blank").await
        .map_err(to_mcp_error)?;

    // goto 内含页面加载等待; 超时通过 chromiumoxide 的 navigation timeout 控制
    // 对 SPA 页面 (React 文档等), 可追加 sleep 等待 JS 渲染
    page.goto(&params.url).await.map_err(to_mcp_error)?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await; // JS 渲染等待

    // 获取渲染后 HTML
    let html = page.content().await.map_err(to_mcp_error)?;
    page.close().await.ok();

    if html.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(
            format!("Failed to load content from {}", params.url)
        )]));
    }

    // dom_smoothie 正文提取
    let extracted = ContentExtractor::extract(&html, &params.url, params.max_length)
        .map_err(|e| McpError::new(ErrorCode::INTERNAL_ERROR, format!("Content extraction failed: {}", e)))?;

    let text = format!(
        "# {}\n\nSource: {}\n\n---\n\n{}",
        extracted.title, params.url, extracted.content
    );

    Ok(CallToolResult::success(vec![Content::text(text)]))
}
```

### 验收标准

- [ ] `read_page(url="https://doc.rust-lang.org/book/ch01-01-installation.html")` 返回 Markdown 正文
- [ ] 返回内容包含文章标题、正文段落、代码块
- [ ] 返回内容**不包含**导航栏、侧边栏、页脚、广告
- [ ] `max_length=500` 时正确截断并附加 "(... content truncated)"
- [ ] 导航超时(15s) 返回 MCP 错误而非 hang
- [ ] 空页面/404 页面返回友好提示而非崩溃
- [ ] JS 重度页面（如 React 文档）可正确渲染后提取

---

## T6: 集成测试与交付

> 关联：ARCHITECTURE.md §2.1 Token 效率对比, §2.2 平台兼容性, §8.2 各平台接入

### 进入标准

以下条件**全部满足**后方可进入 T6：

- [ ] T4 所有验收标准通过
- [ ] T5 所有验收标准通过
- [ ] `cargo build --release` 成功，无 warning
- [ ] `cargo clippy` 无 error
- [ ] 单独测试 web_search 和 read_page 各至少 3 次无崩溃

### 测试矩阵

**6.1: 功能测试**

| 编号 | 测试场景 | 命令/操作 | 期望结果 |
|------|---------|----------|---------|
| F1 | 搜索技术文档 | `web_search("tokio async runtime tutorial")` | ≥5 条结果，含 tokio.rs 相关链接 |
| F2 | 搜索错误排障 | `web_search("rust lifetime error cannot infer")` | ≥5 条结果，含 Stack Overflow/Reddit |
| F3 | 阅读 Rust 文档 | `read_page("https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html")` | Markdown 正文，含代码块 |
| F4 | 阅读 GitHub README | `read_page("https://github.com/nickel-org/rust-by-example")` | 项目描述 Markdown |
| F5 | 搜索引擎指定 | `web_search("test", engine="bing")` | 使用 Bing 搜索 |
| F6 | 空查询 | `web_search("")` | 合理的空结果或错误提示 |
| F7 | 不存在的 URL | `read_page("https://example.com/nonexistent-page-404")` | 友好错误提示 |

**6.2: Chrome 连接模式测试**

| 编号 | 模式 | 测试方法 | 期望 |
|------|------|---------|------|
| C1 | 复用用户 Chrome | 开启 `--remote-debugging-port=9222`，运行 `ailonk-search` | 连接成功，Google 搜索正常 |
| C2 | Headless 自动启动 | 不开 Chrome，运行 `ailonk-search` | 自动启动 headless，Bing 搜索正常 |
| C3 | Headless 强制 | 运行 `ailonk-search --headless` | 启动 headless |
| C4 | 无 Chrome | 移除 Chrome 路径后运行 | 清晰错误提示 |

**6.3: 平台集成测试**

| 编号 | 平台 | 配置 | 测试 |
|------|------|------|------|
| P1 | Cursor | `~/.cursor/mcp.json` 添加配置 | AI 可调用 web_search 和 read_page |
| P2 | Claude Code | `.mcp.json` 添加配置 | AI 可调用工具 |

**6.4: Token 效率验证**

| 编号 | 场景 | 方法 | 目标 |
|------|------|------|------|
| E1 | web_search 返回 10 条结果 | 计算返回文本 token 数 | < 800 tokens |
| E2 | read_page 阅读一篇文章 | 计算返回文本 token 数 | < 5000 tokens |
| E3 | 对比 Chrome DevTools MCP | 同一搜索任务对比总 token | 节省 ≥ 3 倍 (nice-to-have, 定性验证) |

### P0 MVP 交付标准

| 标准 | 状态 |
|------|------|
| `cargo build --release` 成功 | |
| `cargo clippy` 无 error | |
| 功能测试 F1-F7 全部通过 | |
| 连接测试 C1-C3 全部通过 (C4 为 nice-to-have) | |
| 至少 1 个平台集成测试 (P1 或 P2) 通过 | |
| Token 效率 E1-E2 达标 | |
| 无内存泄漏 (headless 运行 10 分钟后 RSS 稳定) | |
| README.md 包含安装和配置说明 | |

---

## 附录：P1 预览

P0 交付后，P1 的核心任务：

| 任务 | 说明 |
|------|------|
| Tab 池 | 替换 P0 的单 Tab 串行为 Tab Pool 并发 (ARCHITECTURE.md §3.4) |
| batch_read | 并发多 URL 抓取 (ARCHITECTURE.md §5.3) |
| search_and_read | 组合 web_search + batch_read (ARCHITECTURE.md §5.4) |
| moka 缓存 | URL → content 缓存，TTL 可配 (ARCHITECTURE.md §7.2) |
| 引擎降级 | CAPTCHA 触发时自动切换引擎 (ARCHITECTURE.md §4.3) |

P1 将在 P0 验证通过后详细拆分。

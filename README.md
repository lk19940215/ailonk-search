**中文** | [English](README_EN.md)

# ailonk-search

**基于真实 Chrome 浏览器的 MCP 搜索服务器 — 让 AI 助手像人一样搜索和阅读网页。**

`ailonk-search` 是一个 Rust 编写的 [MCP (Model Context Protocol)](https://modelcontextprotocol.io) 服务器，通过 Chrome DevTools Protocol (CDP) 驱动真实浏览器，为 AI 代理提供网页搜索和内容提取能力。内置反爬虫保护、智能页面加载检测和 Token 高效的 Markdown 输出。

---

## 核心特性

- **5 个 MCP 工具** — `search_and_read`（推荐）、`web_search`、`read_page`、`batch_read`、`screenshot`
- **反爬虫保护** — 基于 [eoka](https://crates.io/crates/eoka)：二进制补丁 + 指纹一致性 + 类人鼠标/键盘操作
- **双运行模式** — **Headless**（零配置，适合服务器/CI）和 **UserChrome**（保留登录态、Cookie、扩展）
- **ML 正文提取** — [rs-trafilatura](https://crates.io/crates/rs-trafilatura) 支持 7 种页面类型（文章、博客、新闻、产品、论坛、文档、通用）
- **智能等待** — 自动识别 SSR/静态页面 vs SPA/动态渲染页面，按需等待
- **Tab 池 + 缓存** — 信号量限制并发 Tab 数 + [moka](https://crates.io/crates/moka) 内容缓存（默认 TTL 300s）
- **懒加载 Chrome** — 首次工具调用时才启动浏览器，`tools/list` 不触发启动
- **多搜索引擎** — Google、Bing、DuckDuckGo，中国区自动选择 cn.bing.com
- **自动检测浏览器** — 支持 Google Chrome、Microsoft Edge、Chromium（macOS/Linux/Windows）

---

## 快速开始

### 安装

**通过 npm（推荐，无需 Rust 环境）：**

```bash
npx ailonk-search --help
# 或全局安装
npm install -g ailonk-search
```

**从 GitHub Releases 下载：**

```bash
# macOS (Apple Silicon)
curl -L https://github.com/lk19940215/ailonk-search/releases/latest/download/ailonk-search-aarch64-apple-darwin \
  -o /usr/local/bin/ailonk-search && chmod +x /usr/local/bin/ailonk-search

# macOS (Intel)
curl -L https://github.com/lk19940215/ailonk-search/releases/latest/download/ailonk-search-x86_64-apple-darwin \
  -o /usr/local/bin/ailonk-search && chmod +x /usr/local/bin/ailonk-search

# Linux (x86_64)
curl -L https://github.com/lk19940215/ailonk-search/releases/latest/download/ailonk-search-x86_64-unknown-linux-gnu \
  -o /usr/local/bin/ailonk-search && chmod +x /usr/local/bin/ailonk-search
```

**从源码编译（需要 Rust 1.91+）：**

```bash
git clone https://github.com/lk19940215/ailonk-search.git
cd ailonk-search
cargo install --path .
```

### 首次配置（UserChrome 模式）

如果需要访问登录态页面（GitHub、知乎、付费墙网站等），先运行一次配置：

```bash
ailonk-search setup
```

按提示操作，会创建一个独立的调试 Chrome Profile（端口 19222），不影响正常浏览器使用。

### MCP 配置

将 `ailonk-search` 添加到你的 AI 工具的 MCP 配置中。

#### Cursor

编辑 `~/.cursor/mcp.json`（或项目目录 `.cursor/mcp.json`）：

```json
{
  "mcpServers": {
    "ailonk-search": {
      "command": "npx",
      "args": ["-y", "ailonk-search"]
    }
  }
}
```

#### Claude Code

编辑 `~/.claude/settings.json` 或项目 `.mcp.json`：

```json
{
  "mcpServers": {
    "ailonk-search": {
      "command": "npx",
      "args": ["-y", "ailonk-search"]
    }
  }
}
```

#### Codex / OpenAI CLI

```json
{
  "ailonk-search": {
    "command": "npx",
    "args": ["-y", "ailonk-search", "--headless"]
  }
}
```

**Headless 模式（无需 setup，适合服务器/CI）：**

```json
"args": ["-y", "ailonk-search", "--headless"]
```

**UserChrome 模式（运行 `setup` 后使用，保留登录态）：**

```json
"args": ["-y", "ailonk-search"]
```

**自定义浏览器路径 / 区域：**

```json
"args": ["-y", "ailonk-search", "--chrome-path", "/path/to/chrome", "--region", "cn"]
```

---

## 工具说明

| 工具 | 说明 | 主要参数 |
|------|------|---------|
| `search_and_read` | **推荐**。搜索并自动读取排名靠前的结果，一次调用完成 | `query`, `engine`, `search_count`, `read_count`, `max_length_per_page` |
| `web_search` | 仅搜索，返回标题、URL、摘要列表 | `query`, `engine`, `count` |
| `read_page` | 读取指定 URL，提取为 Markdown | `url`, `include_links`, `max_length` |
| `batch_read` | 并发读取最多 10 个 URL | `urls`, `max_length_per_page`, `concurrency` |
| `screenshot` | 截图（返回 base64 或保存文件）。文本内容请用 `read_page` | `url`, `format`, `file_path` |

**推荐工作流**：`search_and_read` → `read_page`（深入特定 URL）→ `web_search`（仅需结果列表时）

---

## 配置参数

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--remote-url` | — | 连接已有的 Chrome CDP 端点 |
| `--headless` | false | 强制使用 Headless Chrome |
| `--chrome-path` | 自动检测 | Chrome/Edge/Chromium 可执行文件路径 |
| `--engine` | `auto` | 默认搜索引擎：`auto`、`google`、`bing` |
| `--region` | `auto` | 搜索区域：`auto`（自动检测）、`cn`（中国大陆）、`global` |
| `--max-tabs` | `5` | 最大并发 Tab 数 |
| `--chrome-args` | — | 额外 Chrome 启动参数（逗号分隔） |
| `--cache-ttl` | `300` | 内容缓存 TTL（秒），`0` 禁用 |
| `--allow-private-urls` | false | 允许访问内网/本地 URL（127.0.0.1、192.168.* 等） |

### 子命令

| 命令 | 说明 |
|------|------|
| `ailonk-search` / `serve` | 作为 MCP 服务器运行（默认） |
| `setup` | 首次 UserChrome Profile 配置 |
| `cleanup` | 清理调试 Profile 并打印恢复步骤 |
| `test-all` | 运行所有测试场景 |
| `test-search` | 开发：直接测试搜索 |
| `test-read` | 开发：直接测试页面读取 |
| `test-search-and-read` | 开发：测试搜索+读取流程 |

---

## 运行模式

### Headless 模式

- 设置 `--headless` 时自动启用，或无 UserChrome Profile 时自动回退
- 使用 eoka 隐身：二进制补丁 + 类人鼠标/键盘模拟
- 适合：服务器、CI、无界面环境、零配置使用
- 默认引擎：**Bing**（Headless 下更可靠）
- 请求间隔：5 秒

### UserChrome 模式

- 连接（或启动）真实 Chrome 实例，使用 `~/.ailonk-search-profile` 配置
- 保留登录态、Cookie、浏览器扩展
- 使用调试端口 **19222**，与正常 Chrome 共存
- 适合：需要登录的网站、验证码频繁的页面、非中国区 Google 搜索
- 默认引擎：**Google**（全球）或 **Bing cn**（中国）
- 请求间隔：2 秒

| 场景 | 推荐模式 |
|------|---------|
| 服务器 / CI / 快速体验 | Headless (`--headless`) |
| 需要登录的网站（GitHub、知乎等） | UserChrome（先运行 `setup`） |
| 中国大陆搜索 | UserChrome 或 `--region cn` |
| 连接已有 Chrome | `--remote-url http://127.0.0.1:9222` |

---

## 开发

### 构建

```bash
cargo build
cargo build --release
```

### 本地运行（stdio MCP）

```bash
cargo run -- --headless                    # Headless 模式
cargo run --                               # UserChrome 模式（需先 setup）
RUST_LOG=ailonk_search=debug cargo run -- --headless  # 调试日志
```

### 测试

```bash
# 协议层测试（快速，不需要 Chrome）
cargo test --test mcp_integration -- --test-threads=1

# 完整 E2E 测试（需要 Chrome + 网络）
cargo test --test mcp_integration -- --ignored --test-threads=1

# 开发测试
cargo run -- test-search "Rust 异步编程"
cargo run -- --headless test-read "https://example.com"
cargo run -- --headless test-search-and-read "MCP protocol" --read-count 2
cargo run -- test-all
```

### 项目结构

```
src/
├── server/       # MCP 工具定义和处理器
├── browser/      # Chrome CDP 管理、Tab 池、页面导航
├── search/       # Google、Bing、DuckDuckGo 搜索引擎
├── extract/      # rs-trafilatura 正文提取
├── cache/        # moka 内容缓存
└── commands/     # CLI 子命令（serve、setup、test）
```

---

## License

MIT — 查看 [LICENSE](LICENSE)

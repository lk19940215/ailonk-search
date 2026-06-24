**中文** | [English](README_EN.md)

# ailonk-search

**基于真实 Chrome 浏览器的 MCP 搜索服务器 — 让 AI 助手像人一样搜索和阅读网页。**

`ailonk-search` 是一个 Rust 编写的 [MCP (Model Context Protocol)](https://modelcontextprotocol.io) 服务器，通过 Chrome DevTools Protocol (CDP) 驱动真实浏览器，为 AI 代理提供网页搜索和内容提取能力。内置反爬虫保护、智能页面加载检测和 Token 高效的 Markdown 输出。

---

## 核心特性

- **7 个 MCP 工具** — `search_and_read`（推荐）、`web_search`、`read_page`、`batch_read`、`screenshot`、`click_authorize`、`sync_login`
- **反爬虫保护** — 基于 [eoka](https://crates.io/crates/eoka)：二进制补丁 + 指纹一致性 + 类人鼠标操作；搜索输入通过 CDP insertText 一次性插入（~50x 提速）
- **Google snippet 提取** — 结构化 DOM fallback，搜索摘要更可靠
- **三种运行模式** — **AutoConnect** ⭐（Chrome 144+，直接使用主浏览器所有登录态）、**UserChrome**（独立调试 Profile）、**Headless**（零配置）
- **ML 正文提取** — [rs-trafilatura](https://crates.io/crates/rs-trafilatura) 支持 7 种页面类型（文章、博客、新闻、产品、论坛、文档、通用）
- **智能等待** — network idle + MutationObserver DOM 稳定检测，精确等待 SPA 渲染完成
- **内容质量门控** — CAPTCHA/登录墙检测 + `rs-trafilatura` 质量评分，低质量内容标记 `[READ_FAILED]` 且不缓存
- **Tab 池 + 缓存** — 信号量限制并发 Tab 数 + [moka](https://crates.io/crates/moka) 内容缓存（默认禁用，可通过 `--cache-ttl` 启用）
- **懒加载 Chrome** — 首次工具调用时才启动浏览器，`tools/list` 不触发启动
- **多搜索引擎** — Google、Bing、DuckDuckGo，中国区自动选择 cn.bing.com
- **自动检测浏览器** — 支持 Google Chrome、Microsoft Edge、Chromium（macOS/Linux/Windows）

---

## 快速开始

### 1. 安装

**通过 npm（推荐，无需 Rust 环境）：**

```bash
npx ailonk-search --help
# 或全局安装
npm install -g ailonk-search
```

安装器会自动从 GitHub Releases 获取最新版本（不绑定 npm 包版本），支持 3 次重试、60s 超时和下载进度显示。

**环境变量（可选）：**

```bash
# 中国用户：通过镜像加速下载和版本检测
GITHUB_MIRROR=https://ghproxy.com npm install -g ailonk-search

# 固定安装指定版本
AILONK_VERSION=0.1.5 npm install -g ailonk-search
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

### 2. 选择运行模式

| 模式 | 适用场景 | 配置步骤 | 登录态 |
|------|---------|---------|--------|
| **AutoConnect** ⭐ | 需要登录态（推荐，Chrome 144+） | 在 Chrome 中开启一个开关 | 主 Chrome 所有登录态即时可用 |
| **UserChrome** | 需要登录态，但不想动主 Chrome | 运行 `setup` | 需手动登录或 `sync` |
| **Headless**（默认 fallback） | 服务器、CI、快速体验 | 零配置 | 无 |

**选择 AutoConnect 模式**（推荐，所有登录态即时可用）：

1. 在 Chrome 中打开 `chrome://inspect/#remote-debugging`，勾选"启用远程调试"（一次性）
2. 完成！ailonk-search 会自动连接你的主 Chrome

> 首次连接时 Chrome 会弹出权限对话框，点击"允许"即可。

**选择 UserChrome 模式**（独立调试 Profile）：

```bash
# npm 安装用户：
npx ailonk-search setup

# GitHub Releases / 源码编译用户：
ailonk-search setup
```

`setup` 会创建独立的调试 Chrome Profile（端口 19222），不影响正常浏览器使用。登录态过期时可运行 `ailonk-search sync` 或通过 MCP 工具 `sync_login` 自动刷新。

**选择 Headless 模式**（零配置，直接跳到第 3 步）：

```bash
# 不需要任何额外操作，Headless 模式开箱即用
```

### 3. 配置 MCP

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

> 上述配置会自动检测：有 Profile → UserChrome 模式，无 Profile → Headless 模式。
> 若需强制 Headless：`"args": ["-y", "ailonk-search", "--headless"]`

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

**其他配置示例：**

```json
// 强制 Headless 模式
"args": ["-y", "ailonk-search", "--headless"]

// 自定义浏览器路径 + 中国区搜索
"args": ["-y", "ailonk-search", "--chrome-path", "/path/to/chrome", "--region", "cn"]

// 允许访问内网 URL
"args": ["-y", "ailonk-search", "--allow-private-urls"]
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
| `click_authorize` | 自动识别并点击 OAuth/SSO 授权页面（Google OAuth、账号选择、SAML 等） | `url`, `timeout` |
| `sync_login` | 从主 Chrome 同步登录态到调试 Profile（AI 可自动调用） | 无参数 |

**推荐工作流**：`search_and_read` → `read_page`（深入特定 URL）→ `web_search`（仅需结果列表时）

**授权失败处理**：`read_page` 返回 `[READ_FAILED]` 时 → 先尝试 `click_authorize`（处理 OAuth 授权弹窗）→ 若仍失败则 `sync_login`（刷新 Cookie/Session）

### click_authorize — OAuth/SSO 授权

自动识别并点击授权页面上的按钮，完成 OAuth/SSO 登录流程。适用于 `read_page` 因授权拦截返回 `[READ_FAILED]` 的场景。

**适用**：Google OAuth 同意页、Google 账号选择、Google SAML SSO、企业自定义 SSO、通用登录页（含可检测的登录/SSO 按钮）、多步授权流程（如 SSO → Google → 回跳）

**不适用**：Cookie/Session 过期（请用 `sync_login`）、用户名密码登录、CAPTCHA、多因素认证

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `url` | string | 必填 | 需要授权的页面 URL |
| `timeout` | uint | 30 | 等待授权流程完成的最长时间（秒） |

**流程**：导航到 URL → 检测授权类型 → 点击按钮 → 等待 Chrome 原生处理 → 重定向完成

**示例**：

```json
{
  "tool": "click_authorize",
  "arguments": {
    "url": "https://docs.google.com/document/d/abc123/edit",
    "timeout": 30
  }
}
```

**典型工作流**：

```
1. read_page("https://docs.google.com/...")  →  [READ_FAILED]（OAuth 拦截）
2. click_authorize("https://docs.google.com/...")  →  授权成功
3. read_page("https://docs.google.com/...")  →  返回正文 Markdown
```

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
| `--cache-ttl` | `0` | 内容缓存 TTL（秒），`0` 禁用，推荐 `300` 启用 |
| `--allow-private-urls` | false | 允许访问内网/本地 URL（127.0.0.1、192.168.* 等） |

### 子命令

| 命令 | 说明 |
|------|------|
| `ailonk-search` / `serve` | 作为 MCP 服务器运行（默认） |
| `setup` | 创建 UserChrome Profile（独立调试实例） |
| `sync` | 同步登录态（Cookies + localStorage + sessionStorage） |
| `cleanup` | 清理调试 Profile 并打印恢复步骤 |
| `test-all` | 运行所有测试场景 |
| `test-search` | 开发：直接测试搜索 |
| `test-read` | 开发：直接测试页面读取 |
| `test-search-and-read` | 开发：测试搜索+读取流程 |

---

## 运行模式

### AutoConnect 模式 ⭐（Chrome 144+）

- 自动检测 Chrome 的 `DevToolsActivePort` 并通过 WebSocket 直连
- **所有登录态即时可用**，无需 sync、无独立 Profile
- 一次性配置：在 Chrome 中启用 `chrome://inspect/#remote-debugging`
- 首次连接时 Chrome 弹出权限对话框（点击"允许"）
- 默认引擎：**Google**（全球）或 **Bing cn**（中国）
- 请求间隔：2 秒

### UserChrome 模式

- 连接（或启动）真实 Chrome 实例，使用 `~/.ailonk-search-profile` 独立配置
- 通过 `sync` 命令或 MCP `sync_login` 工具从主 Chrome 同步登录态
- 使用调试端口 **19222**，与正常 Chrome 共存
- 默认引擎：**Google**（全球）或 **Bing cn**（中国）
- 请求间隔：2 秒

### Headless 模式

- 设置 `--headless` 时自动启用，或无 UserChrome Profile 时自动回退
- 使用 eoka 隐身：二进制补丁 + 类人鼠标/键盘模拟
- 适合：服务器、CI、无界面环境、零配置使用
- 默认引擎：**Bing**（Headless 下更可靠）
- 请求间隔：5 秒

### 连接优先级

ailonk-search 按以下顺序尝试连接：

1. `--remote-url`（显式指定）
2. `DevToolsActivePort`（AutoConnect，Chrome 144+）
3. 端口 19222（UserChrome，`setup` 创建的实例）
4. 自动启动 Chrome（UserChrome 或 Headless fallback）

| 场景 | 推荐模式 |
|------|---------|
| 需要登录态的网站 | **AutoConnect**（`chrome://inspect` 启用） |
| 不想动主 Chrome | UserChrome（先运行 `setup`） |
| 服务器 / CI / 快速体验 | Headless (`--headless`) |
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
├── server/          # MCP 工具定义和处理器
├── browser/
│   ├── interaction/ # 页面交互层（导航、CAPTCHA、授权、弹窗、Cookie consent）
│   ├── manager.rs   # Chrome 连接管理（AutoConnect/UserChrome/Headless）
│   ├── pool.rs      # Tab 池（信号量限制并发）
│   └── profile.rs   # 登录态 Profile 管理
├── search/          # Google、Bing、DuckDuckGo 搜索引擎
├── extract/         # rs-trafilatura 正文提取
├── cache/           # moka 内容缓存
└── commands/        # CLI 子命令（serve、setup、sync、test）
```

---

## License

MIT — 查看 [LICENSE](LICENSE)

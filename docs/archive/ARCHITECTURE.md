# Chrome Search MCP — 架构设计文档

> 基于 Chrome CDP 直连的 MCP Server，为 Claude Code / Cursor / Codex 等 AI 工具提供「即插即用」的联网搜索与内容抓取能力。

## 1. 背景与问题

### 1.1 第三方 API 联网能力缺失

Claude Code、Cursor、Codex 等 AI 编码工具在使用第三方 API（OpenRouter、DeepSeek、Coding Plan 等）时，**无法使用原生联网搜索**：

- Claude Code 的 `WebSearch`/`WebFetch` 是 Anthropic 服务端工具，第三方 API 请求不经过 Anthropic 服务器，物理上不可用
- Codex（OpenAI）有自己的工具体系，MCP 是扩展的标准方式
- Cursor 使用第三方 API 时同样缺少内置搜索
- `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` 标志是症状而非原因——真正的问题是第三方 API 不实现这些工具

### 1.2 现有方案的实际体验

经实测，现有替代方案均有明显不足：

| 方案 | 实测问题 |
|------|---------|
| **Tavily / Firecrawl MCP** | 付费，有额度限制，API 调用有延迟 |
| **DuckDuckGo MCP** | 对大陆用户不友好（访问受限），结果质量一般 |
| **SearXNG MCP** | 需自建服务，维护成本高 |
| **Chrome DevTools MCP** | **重度使用后的核心痛点**：token 效率极低（每次搜索需 5-10 次工具调用，A11Y 树快照消耗 15000-25000 tokens），速度慢 |

**核心痛点：Token 效率。** Chrome DevTools MCP 虽免费且能力强，但为通用浏览器自动化设计，完成一次搜索+阅读需要多次工具调用和大量 A11Y 树 token，这正是本项目要解决的问题。

### 1.3 Chrome DevTools MCP 的瓶颈

当前架构链路：
```
AI Agent → stdio/JSON-RPC → chrome-devtools-mcp (Node.js) → Puppeteer → CDP WebSocket → Chrome
```

核心瓶颈：

| 瓶颈 | 说明 |
|------|------|
| 串行互斥锁 | MCP 内部 mutex，所有工具调用排队，无法并发 |
| Puppeteer 开销 | 中间多一层 JS 抽象，增加延迟和内存占用 |
| 无障碍树快照 | `take_snapshot` 返回整棵 A11Y 树，token 消耗巨大 |
| 单结果范式 | 每次工具调用只操作一个页面/元素 |
| 通用设计 | 为"浏览器自动化"设计，缺少搜索/采集高层抽象 |

### 1.4 竞品分析与差异化

经深度调研，GitHub 上已有类似方向的项目，但均不能替代我们的目标场景：

| 项目 | 语言 | 搜索能力 | 与我们的差距 |
|------|------|---------|------------|
| [kodegen-tools-browser](https://github.com/cyrup-ai/kodegen-tools-browser) | Rust | `web_search` (仅 DDG) | 绑死 KODEGEN 生态，需 nightly，全局单例 Browser，无 Tab 池/Readability |
| [zavora-ai/mcp-browser](https://github.com/zavora-ai/mcp-browser) | Rust | **无** | 纯浏览器自动化/抓取，不碰搜索，方向不同 |
| [AgentWebSearch-MCP](https://github.com/insung8150/agentwebsearch-mcp) | Python | `web_search`/`smart_search` | Python 非单二进制；3 个有头 Chrome 进程；2026-02 后停更 |
| [MCPSearch](https://github.com/jonusnattapong/mcpsearch) | Python | 多引擎 + 社交 | 依赖重，部署复杂 |
| [scrapelab/mcp](https://github.com/scrapelab/mcp) | Python | 无专用搜索 | 84 工具，太重，反检测导向 |

**我们的差异化：**
1. **搜索优先** — 5 个高层工具，非 17-84 个低层浏览器自动化工具
2. **`search_and_read` 杀手工具** — 1 次调用 = 搜索 + 并发读 top N（竞品需多步）
3. **Tab 池单 Chrome 并发** — 比 3 Chrome 进程更轻，比单例更高效
4. **Token 优化** — Readability + Markdown，非 A11Y 树/原始 HTML
5. **单二进制 + stdio** — 零 Python/Node/nightly/daemon 依赖
6. **定位清晰** — 补第三方 API 的 WebSearch 缺口

**结论：不是重复造轮子，不建议 fork 任何一个。**

## 2. 项目定位

**Chrome Search MCP** = Chrome 的低层 CDP 能力 + 高层「中间转换层」→ 面向 AI 的搜索/阅读工具

**核心价值：**
- 一次 MCP 调用 = 完成完整的搜索/阅读任务（而非 5-10 次低层操作）
- Chrome CDP 直连用户的浏览器，复用登录态/Cookie/代理配置
- Token 极致优化：返回提取后的正文 Markdown，非 HTML/A11Y 树
- 单二进制，即插即用，支持 Claude Code / Cursor / Codex / 任何 MCP 兼容工具
- 无额外语言运行时依赖，仅需系统 Chrome/Chromium

### 2.1 Token 效率对比

```
场景：AI 搜索 "React useEffect cleanup" 并阅读一篇相关文章

【Chrome DevTools MCP】
  navigate_page(google.com)         →  ~200 tokens
  take_snapshot()                   →  ~3000-5000 tokens (A11Y 树)
  fill(uid, query)                  →  ~100 tokens
  press_key(Enter)                  →  ~50 tokens
  take_snapshot()                   →  ~5000-8000 tokens (搜索结果 A11Y 树)
  click(uid)                        →  ~100 tokens
  take_snapshot()                   →  ~5000-10000 tokens (文章页 A11Y 树)
  ─────────────────────────────────────────────
  合计: ~15000-25000 tokens, 7 次工具调用

【Chrome Search MCP】
  search_and_read(query, read_count=1)
  返回: 10 条搜索结果 (~500 tokens) + 1 篇正文 (~3000 tokens)
  ─────────────────────────────────────────────
  合计: ~3500 tokens, 1 次工具调用

  Token 节省: 3-5 倍（保守估计）| 调用次数: 7 → 1
  加上减少的工具调用轮次（每轮重复系统提示+工具定义），实际会话总 token 节省可达 5 倍+
```

### 2.2 平台兼容性

| 平台 | MCP 支持 | 可用性 |
|------|---------|--------|
| **Claude Code** | ✅ stdio 原生 | P0 即可用 |
| **Cursor** | ✅ stdio 原生 | P0 即可用 |
| **Codex (OpenAI)** | ✅ MCP 支持 | P0 即可用 |
| **Windsurf** | ✅ MCP 支持 | P0 即可用 |
| **Cline / Continue.dev / Zed** | ✅ MCP 支持 | P0 即可用 |
| **LangGraph / 服务器** | 需 HTTP 传输 | P3 阶段 |
| **macOS / Linux / Windows** | ✅ | 需系统已安装 Chrome |

## 3. 架构设计

### 3.1 整体架构

```
AI Tool (Claude Code / Cursor / Codex / LangGraph)
  │ MCP (stdio 或 Streamable HTTP)
  ▼
┌───────────────────────────────────────────────────┐
│  ailonk-search (Rust 单二进制)                  │
│                                                    │
│  ┌──────────────────────────────────────────────┐  │
│  │ Layer 1: MCP Server                          │  │
│  │ • rmcp 1.7 官方 SDK                           │  │
│  │ • 传输: stdio (默认) / Streamable HTTP (可选)  │  │
│  │ • Tool 注册与分发                              │  │
│  │ • 请求验证 & 响应格式化                         │  │
│  └──────────────┬───────────────────────────────┘  │
│                 │                                   │
│  ┌──────────────▼───────────────────────────────┐  │
│  │ Layer 2: 高层工具 (对 AI 暴露)                │  │
│  │ • web_search      — 联网搜索                  │  │
│  │ • read_page       — 页面正文提取               │  │
│  │ • batch_read      — 批量并发抓取               │  │
│  │ • search_and_read — 搜索+阅读一体化            │  │
│  │ • screenshot      — 页面截图                   │  │
│  └──────────────┬───────────────────────────────┘  │
│                 │                                   │
│  ┌──────────────▼───────────────────────────────┐  │
│  │ Layer 3: 中间转换层                            │  │
│  │ • 搜索流程编排                                 │  │
│  │   (开Tab→导航→等待→提取→关Tab)                 │  │
│  │ • 引擎降级: 按连接模式自动选择 + 降级            │  │
│  │ • CAPTCHA/拦截检测                             │  │
│  │ • 正文提取 (dom_smoothie Readability)          │  │
│  │ • HTML → Markdown (htmd)                      │  │
│  │ • 并发 Tab 池管理                              │  │
│  │ • 结果聚合 & Token 压缩                        │  │
│  │ • 缓存 (moka, 带 TTL)                         │  │
│  │ • 请求限速 + jitter                            │  │
│  └──────────────┬───────────────────────────────┘  │
│                 │                                   │
│  ┌──────────────▼───────────────────────────────┐  │
│  │ Layer 4: CDP 直连层                            │  │
│  │ • chromiumoxide 0.9 (async CDP 客户端)         │  │
│  │ • Browser + Handler 事件循环 (tokio task)      │  │
│  │ • WebSocket 直连 Chrome                        │  │
│  │ • 多 Target 并发 (无 mutex)                    │  │
│  │ • Chrome 生命周期管理                           │  │
│  └──────────────────────────────────────────────┘  │
└──────────────────┬────────────────────────────────┘
                   │ CDP WebSocket
                   ▼
             Chrome 浏览器
             (用户已有 or 自动启动 headless)
```

### 3.2 Chrome 连接策略

支持三种模式，按优先级自动选择：

```
1. --remote-url http://127.0.0.1:9222
   → 连接指定地址的 Chrome（复用已开远程调试的 Chrome）
   → 优势: 复用用户登录态/Cookie/代理，Google 搜索体验最佳
   → 适合: 本地开发场景

2. --auto-connect (默认)
   → 尝试连接 127.0.0.1:9222
   → 发现 Chrome → 直接连接（等同 remote-url，复用用户 Chrome）
   → 未发现 → 自动启动 headless Chrome
   → 搜索引擎: 连接用户 Chrome → 默认 Google；启动 headless → 默认 Bing

3. --headless
   → 自动查找系统 Chrome 路径
   → 启动 headless 实例 (LANG=en_US.UTF-8)
   → MCP Server 退出时自动关闭
   → 搜索引擎: 默认 Bing（大陆可访问）
```

> **注意：** chromiumoxide 要求 Chrome 语言为 English，否则 debug port 解析可能失败。headless 启动时自动设置 `LANG=en_US.UTF-8`。
>
> **为什么复用用户 Chrome 安全？** CDP 直连只是程序化控制用户的浏览器。Google 看到的仍然是用户自己的浏览器（有登录态、浏览历史、正常指纹），反爬风险极低。只需加合理请求间隔即可。

### 3.3 Browser Manager 与 CDP Handler

chromiumoxide 的 `Browser::launch()` 返回 `(browser, handler)`，必须在后台 tokio task 中持续驱动 handler 事件循环：

```
BrowserManager 职责：
1. Browser::launch() → spawn handler loop (独立 tokio task)
2. handler.next().await 持续驱动 WebSocket 事件
3. Arc<Browser> 共享给 Tab Pool（Browser 不实现 Clone，用 Arc 包装）
4. Tab Pool 通过 browser.new_page() 创建新 Tab（并发安全）
5. MCP 退出时 → browser.close() + handler task join
```

### 3.4 Tab 池并发模型

```
┌─────────────────────────────────┐
│  Tab Pool (默认 max=5)           │
│                                  │
│  ┌─────┐ ┌─────┐ ┌─────┐       │
│  │Tab 1│ │Tab 2│ │Tab 3│  ...   │
│  │搜索 │ │抓取 │ │抓取 │        │
│  └─────┘ └─────┘ └─────┘       │
│                                  │
│  • acquire() → 获取可用 Tab      │
│  • release() → 归还 Tab          │
│  • 超过 max → 排队等待 (超时5s)  │
│  • 空闲 > 60s → close tab 回收   │
│  • Tab 复用: release 时 reset    │
│  •   导航到 about:blank          │
└─────────────────────────────────┘
```

关键设计：
- 每个 Tab 是独立的 CDP Target，天然并发，无需全局 mutex
- `effective_concurrency = min(request.concurrency, max_tabs, urls.len())`
- Tab acquire 超时默认 5s，超时返回错误而非无限等待

### 3.5 内容提取管线

Token 效率的核心在于两层过滤：

**第一层：搜索结果过滤（排除广告）**

CDP `evaluate` JS 只提取自然搜索结果，跳过广告/知识面板/侧栏：
```
Google: div.g (排除 [data-text-ad]、.commercial-unit)
Bing:   li.b_algo (排除 .b_ad、.b_adSlug)
DDG:    div.result / div.web-result
```

**第二层：文章正文提取（排除页面噪音）**

`dom_smoothie` (Mozilla Readability 算法) 对每个 DOM 元素计算内容分数（文本密度、链接比、标签类型），自动排除：
- 导航栏、页脚、侧栏（链接密度高、文本密度低）
- 广告、弹窗、Cookie 横幅
- 评论区、推荐文章、社交分享按钮

只保留标题、正文段落、代码块、列表等核心内容。

**完整管线：**
```
                     搜索场景                          阅读场景
                       │                                │
            CDP evaluate JS                   CDP page.content()
            (只取自然结果)                       (获取完整 HTML)
                       │                                │
                 scraper 解析                    dom_smoothie
              (CSS selector 提取                (Readability 算法
               title/url/snippet)               → TextMode::Markdown
                       │                         自动排除噪音)
                       │                                │
                       ▼                                ▼
              搜索结果纯文本                      正文 Markdown
              (~50 tokens/条)                  (~3000-5000 tokens/页)
```

**底层依赖关系：**
```
web_search       → chromiumoxide (CDP) + scraper (HTML 解析)
read_page        → chromiumoxide (CDP) + dom_smoothie (Readability → MD)
batch_read       → read_page × N (并发 tokio task)
search_and_read  → web_search + batch_read (组合)
screenshot       → chromiumoxide (CDP 截图 API)
缓存层           → moka (所有读操作共享, TTL 避免重复抓取)
```

## 4. 搜索引擎策略

### 4.1 反爬现状评估 (2025-2026)

| 引擎 | 反爬强度 | 说明 |
|------|---------|------|
| **Google** | ★★★★★ 极高 | SearchGuard/BotGuard 2025.1 全面上线，headless 10-50 次请求内即失效，SERP DOM 频繁变化 |
| **Bing** | ★★★☆☆ 中高 | Microsoft Bot Protection + Cloudflare，单 IP 约 5-10 次/小时，机房 IP 约 80% 首请求即封 |
| **DuckDuckGo** | ★★☆☆☆ 低 | html.duckduckgo.com 无 JS 依赖，IP 信誉 + Header 一致性检测，阈值宽松 |

### 4.2 默认引擎策略（按连接模式自动选择）

引擎选择与 Chrome 连接模式强相关：

| 连接模式 | 默认引擎 | 理由 |
|---------|---------|------|
| **复用用户 Chrome** (`--auto-connect` / `--remote-url`) | **Google** | 用户 Chrome 有登录态/Cookie/代理配置，Google 视为真实用户，反爬风险极低 |
| **Headless 新实例** | **Bing** | 大陆可直接访问，无需代理，技术内容质量可接受 |

> **大陆用户说明：** DuckDuckGo 在大陆访问受限，不作为默认引擎。复用用户 Chrome 时（通常已有代理配置），Google 是最佳选择；headless 模式下 Bing 更适合。

### 4.3 引擎降级链

**复用用户 Chrome 时：**
```
google → bing → 返回错误
```

**Headless 模式：**
```
bing → duckduckgo (html 版) → 返回错误
```

触发条件：
- 检测到 `challenge-form` / 空结果集 / HTTP 403/429
- 同一 query 最多降级 1 次，避免雪崩
- **不在 headless 模式下自动切 Google**（几乎必失败）

### 4.4 可扩展引擎（P2+）

通过 `SearchEngine` trait 实现引擎可插拔，后续可扩展：
- **百度** — 中文技术内容补充
- **DuckDuckGo** (html 版) — 隐私友好场景
- **SearXNG** — 自托管实例
- **Brave Search** — 免费 API

### 4.5 搜索结果解析策略

| 引擎 | 解析方式 | 选择器 |
|------|---------|--------|
| **DDG html** | CDP 导航后 `page.content()` → Rust `scraper` 解析 | `div.result` 或 `div.web-result`；URL 从 `uddg=` 参数解码 |
| **DDG 主站** | CDP JS `evaluate` | `[data-testid='result']` |
| **Bing** | CDP JS + 字段级容错 | `li.b_algo`；URL 从 base64 `u` 参数解码 |
| **Google** | CDP JS + 字段级容错 | `div.g` / `h3` + `a[href]`（布局 1-2 次/年会变） |

**原则：**
- 双 selector fallback：如 DDG 的 `div.result` + `div.web-result`
- 字段级 try/catch：单个 selector 失效不拖垮整页
- URL 清洗层独立：DDG `uddg`、Bing `bing.com/ck/a` redirect 统一处理

### 4.6 MVP 必须实现的反爬措施

| 优先级 | 措施 | 说明 |
|--------|------|------|
| **P0** | 请求限速 + jitter | 搜索间隔 2-5s 随机 |
| **P0** | 完整浏览器 Header | UA 与 Chrome 版本一致；补 `Accept-Language`、`sec-fetch-*` |
| **P0** | CAPTCHA/拦截检测 | 空结果、`challenge-form`、异常 title → 触发降级 |
| **P0** | 引擎 auto 选择 | 复用 Chrome → Google；headless → Bing |
| **P1** | 会话 cookie 复用 | 同 Tab 内保持 cookie |
| **P2** | UA 轮换 | 对 DDG/Bing 有帮助 |

## 5. MCP Tools 设计

### 5.1 web_search — 联网搜索

**场景：** AI 需要搜索信息（文档、教程、错误解决方案等）

```json
{
  "name": "web_search",
  "description": "Search the web using a real Chrome browser. Returns multiple results with titles, URLs, and snippets in a single call.",
  "inputSchema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query"
      },
      "engine": {
        "type": "string",
        "enum": ["auto", "google", "bing", "duckduckgo"],
        "default": "auto",
        "description": "Search engine. 'auto' selects based on Chrome connection mode: Google for user Chrome, Bing for headless."
      },
      "count": {
        "type": "integer",
        "default": 10,
        "minimum": 1,
        "maximum": 20,
        "description": "Number of results to return"
      }
    }
  }
}
```

**内部执行流程：**
```
1. 确定引擎: auto → 检查连接模式 → Google(用户Chrome) 或 Bing(headless)
2. Tab Pool → acquire Tab (超时 5s)
3. 导航到搜索引擎 URL (google.com/search?q=... 或 bing.com/search?q=...)
4. 等待搜索结果加载 (wait for selector, 超时 10s)
5. evaluate JS 提取搜索结果 (按引擎选择不同选择器)
6. CAPTCHA/拦截检测 → 命中则触发引擎降级
7. 格式化返回
8. release Tab
```

**返回格式（token 优化纯文本）：**
```
Found 10 results for "Go chromedp tutorial":

1. [Getting Started with chromedp](https://example.com/chromedp-guide)
   A comprehensive guide to browser automation in Go using chromedp...

2. [chromedp/examples - GitHub](https://github.com/chromedp/examples)
   Repository of chromedp usage examples including screenshots, scraping...

(... 10 results total)
```

**对比 Chrome DevTools MCP：**
| 操作 | Chrome DevTools MCP | Chrome Search MCP |
|------|--------------------|--------------------|
| 搜索 "chromedp" | 5-10 次工具调用 + 大量 A11Y snapshot token | **1 次调用** |
| Token 消耗 | 高（A11Y 树 + 多次 JSON-RPC） | **低（纯文本结果）** |

### 5.2 read_page — 页面正文提取

```json
{
  "name": "read_page",
  "description": "Fetch a URL using Chrome and extract the main content as clean Markdown. Strips navigation, ads, scripts, and styles.",
  "inputSchema": {
    "type": "object",
    "required": ["url"],
    "properties": {
      "url": {
        "type": "string",
        "description": "URL to read"
      },
      "include_links": {
        "type": "boolean",
        "default": true,
        "description": "Preserve hyperlinks in extracted content"
      },
      "max_length": {
        "type": "integer",
        "default": 15000,
        "description": "Maximum content length in characters"
      }
    }
  }
}
```

**内部执行流程：**
```
1. 检查 moka 缓存 (key = hash(url + include_links + max_length))
   → 命中则直接返回
2. Tab Pool → acquire Tab
3. 导航到 URL，等待加载完成 (超时 15s)
4. page.content() → dom_smoothie 提取正文 (TextMode::Markdown 直出 Markdown)
5. 截断到 max_length
7. 写入 moka 缓存 (TTL = cache-ttl 配置)
8. release Tab
```

### 5.3 batch_read — 批量并发抓取

```json
{
  "name": "batch_read",
  "description": "Fetch multiple URLs concurrently using Chrome tabs and extract main content from each. Returns partial results if some URLs fail.",
  "inputSchema": {
    "type": "object",
    "required": ["urls"],
    "properties": {
      "urls": {
        "type": "array",
        "items": { "type": "string" },
        "maxItems": 10,
        "description": "List of URLs to read"
      },
      "max_length_per_page": {
        "type": "integer",
        "default": 5000,
        "description": "Maximum content length per page"
      },
      "concurrency": {
        "type": "integer",
        "default": 5,
        "minimum": 1,
        "maximum": 10,
        "description": "Maximum parallel Chrome tabs"
      }
    }
  }
}
```

**内部执行流程：**
```
1. effective_concurrency = min(concurrency, max_tabs, urls.len())
2. 启动 N 个并发 tokio task
3. 每个 task:
   a. Tab Pool → acquire Tab
   b. 导航 + 等待 + 提取 + 转换
   c. release Tab
   d. 单个 URL 失败 → 记录 error，不影响其他
4. 等待所有 task 完成
5. 聚合返回: { results: [...], errors: [...] }
```

**部分成功模式：** 即使某些 URL 失败（超时/404），仍返回成功的部分结果 + 错误列表。

### 5.4 search_and_read — 搜索+阅读一体化（杀手级工具）

```json
{
  "name": "search_and_read",
  "description": "Search the web, then automatically fetch and extract content from the top results. Combines web_search + batch_read in a single call.",
  "inputSchema": {
    "type": "object",
    "required": ["query"],
    "properties": {
      "query": {
        "type": "string",
        "description": "Search query"
      },
      "engine": {
        "type": "string",
        "enum": ["auto", "google", "bing", "duckduckgo"],
        "default": "auto"
      },
      "search_count": {
        "type": "integer",
        "default": 10,
        "description": "Number of search results"
      },
      "read_count": {
        "type": "integer",
        "default": 3,
        "minimum": 1,
        "maximum": 5,
        "description": "How many top results to read in full"
      },
      "max_length_per_page": {
        "type": "integer",
        "default": 5000
      }
    }
  }
}
```

**内部执行流程：**
```
1. Tab 1: 自动选择引擎搜索 → 提取 N 条结果
2. Tab 2..M: 并发打开前 read_count 条 URL
3. 每个页面: dom_smoothie 提取正文 → Markdown
4. 聚合返回:
   - 搜索结果摘要 (全部 N 条)
   - 正文内容 (前 read_count 条)
5. release 所有 Tab
```

**返回示例：**
```
# Search Results for "React hooks best practices 2026"

## Results (10 found)

1. [React Hooks Best Practices](https://react.dev/learn/hooks-best-practices) ⭐ read
2. [10 React Hooks Mistakes](https://blog.example.com/hooks-mistakes) ⭐ read
3. [useEffect Complete Guide](https://overreacted.io/useeffect-guide) ⭐ read
4. [React Hooks Cheat Sheet](https://example.com/cheatsheet)
5. ...

---

## [1] React Hooks Best Practices (react.dev)

React Hooks introduced a paradigm shift...

### Rules of Hooks
- Only call Hooks at the top level
...

---

## [2] 10 React Hooks Mistakes (blog.example.com)

Here are the most common mistakes...
...
```

**一次调用 = Chrome DevTools MCP 下 20+ 次工具调用**

### 5.5 screenshot — 页面截图

```json
{
  "name": "screenshot",
  "description": "Take a screenshot of a webpage using Chrome. Returns MCP ImageContent (base64 inline). For large screenshots, use filePath to save to disk.",
  "inputSchema": {
    "type": "object",
    "required": ["url"],
    "properties": {
      "url": { "type": "string" },
      "full_page": { "type": "boolean", "default": false },
      "selector": { "type": "string", "description": "CSS selector to screenshot specific element" },
      "format": { "type": "string", "enum": ["png", "jpeg", "webp"], "default": "png" },
      "file_path": { "type": "string", "description": "Save to file instead of returning inline base64" }
    }
  }
}
```

**返回格式：**
- 无 `file_path` → MCP `ImageContent` (base64 inline)
- 有 `file_path` → 写入文件 + 返回文件路径文本

## 6. 错误处理与超时

### 6.1 超时默认值

| 操作 | 默认超时 | 可配置 |
|------|---------|--------|
| Tab acquire | 5s | `--tab-timeout` |
| 页面导航 | 15s | `--nav-timeout` |
| 搜索结果等待 | 10s | — |
| 单页内容提取 | 20s | — |
| `batch_read` 整体 | 60s | — |

### 6.2 错误格式

所有工具调用失败时返回 MCP 标准错误：

```json
{
  "isError": true,
  "content": [
    {
      "type": "text",
      "text": "Search failed: all engines returned CAPTCHA/block. Fallback chain: google → bing → exhausted. Try --remote-url to connect your Chrome with login state for better results."
    }
  ]
}
```

### 6.3 `batch_read` 部分成功

```
## Successfully read 2/3 pages

### [1] https://example.com/page1 ✅
(content...)

### [2] https://example.com/page2 ✅
(content...)

## Errors (1)
- https://example.com/page3: Navigation timeout after 15s
```

## 7. 技术选型

### 7.1 语言：Rust

| 维度 | 选择理由 |
|------|---------|
| MCP SDK | `rmcp` — 官方 SDK，3500+ ⭐，12M+ 下载，`#[tool]` 宏极大简化开发 |
| CDP 库 | `chromiumoxide` — 1300+ ⭐，async/tokio，生产可用 |
| 分发 | 单二进制，无额外语言运行时依赖，仅需系统 Chrome/Chromium |
| 性能 | 零开销抽象，async/await，适合多 WebSocket 并发 |
| 安全 | 编译时内存安全，适合长期运行的 MCP Server |
| MSRV | Rust 1.85+（chromiumoxide 0.9 要求） |

### 7.2 核心依赖

```toml
[dependencies]
# MCP Server (官方 Rust SDK)
rmcp = { version = "1.7", features = [
    "server",
    "macros",
    "transport-streamable-http-server",  # HTTP 传输 (可选，P3 阶段启用)
] }

# HTTP 挂载 (P3 HTTP 模式需要，非 rmcp feature)
axum = "0.8"

# Chrome CDP 客户端
chromiumoxide = { version = "0.9", features = ["fetcher", "rustls", "zip8"] }
# fetcher: 自动下载 Chrome; rustls: TLS 支持; zip8: 压缩

# 异步运行时
tokio = { version = "1", features = ["full"] }
futures = "0.3"

# HTML 处理
dom_smoothie = { version = "0.18", features = ["serde"] }  # 正文提取 (Mozilla Readability, 内置 Markdown 模式)
htmd = "0.5"           # HTML → Markdown 精细转换
scraper = "0.27"       # CSS 选择器 HTML 解析

# 缓存 (带 TTL + async 支持)
moka = { version = "0.12", features = ["future"] }

# 序列化
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "1"         # JSON Schema 生成 (rmcp tool 参数)

# CLI
clap = { version = "4", features = ["derive"] }

# 日志
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# 错误处理
anyhow = "1"
```

**依赖选型说明：**

| crate | 选择理由 |
|-------|---------|
| `dom_smoothie` 0.18 | 最接近 Mozilla Readability.js 的 Rust 移植，内置 `TextMode::Markdown`，活跃维护 |
| `moka` 0.12 | 替代 `lru`（不支持 TTL），支持 TTL + async + 并发 |
| `chromiumoxide` features | `fetcher` 自动下载 Chrome，`rustls` TLS，`zip8` 解压（**注意：无 `tokio` feature**，tokio 是硬依赖） |

### 7.3 项目结构

```
ailonk-search/
├── Cargo.toml
├── README.md
├── ARCHITECTURE.md              # 本文件
├── Dockerfile
├── src/
│   ├── main.rs                  # 入口：CLI 参数解析 + 启动
│   ├── server/
│   │   ├── mod.rs
│   │   └── tools.rs             # MCP Tool 定义与 handler (#[tool] 宏)
│   ├── browser/
│   │   ├── mod.rs
│   │   ├── manager.rs           # Chrome 生命周期 + Handler 事件循环
│   │   └── pool.rs              # Tab 池，并发控制，空闲回收
│   ├── search/
│   │   ├── mod.rs
│   │   ├── engine.rs            # SearchEngine trait + 引擎自动选择 + 降级链
│   │   ├── google.rs            # Google 搜索结果解析 (P0, 用户 Chrome 模式)
│   │   ├── bing.rs              # Bing 搜索结果解析 (P0, headless 模式)
│   │   └── duckduckgo.rs        # DuckDuckGo 搜索结果解析 (P2 扩展)
│   ├── extract/
│   │   ├── mod.rs
│   │   └── content.rs           # dom_smoothie 正文提取 (TextMode::Markdown)
│   └── cache.rs                 # moka 缓存封装 (TTL + async)
└── tests/
    ├── fixtures/             # 保存的 HTML (单元测试用，不需要 Chrome)
    │   ├── google_results.html
    │   ├── bing_results.html
    │   └── sample_article.html
    ├── search_test.rs
    └── extract_test.rs
```

## 8. 配置与使用

### 8.1 CLI 参数

```bash
# 默认模式：auto-connect (尝试 9222 → fallback headless)
ailonk-search

# 连接已有 Chrome（复用 cookie/登录态，Google 搜索最佳模式）
ailonk-search --remote-url http://127.0.0.1:9222

# 强制 headless（不尝试连接已有 Chrome）
ailonk-search --headless

# 指定 Chrome 路径
ailonk-search --chrome-path "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"

# 配置默认搜索引擎 (auto | google | bing | duckduckgo)
ailonk-search --engine google  # 强制用 Google（需确保 Chrome 可访问 Google）

# Tab 池大小
ailonk-search --max-tabs 8

# 缓存 TTL（秒）
ailonk-search --cache-ttl 300

# HTTP 服务器模式（P3 阶段）
ailonk-search --transport http --port 8080

# HTTP 模式认证 token
ailonk-search --transport http --port 8080 --auth-token "your-secret"

# 透传 Chrome 启动参数
ailonk-search --chrome-args="--disable-dev-shm-usage,--no-sandbox"
```

### 8.2 各平台接入

**Claude Code** (`~/.claude.json` 或项目 `.mcp.json`):
```json
{
  "mcpServers": {
    "web-search": {
      "command": "ailonk-search",
      "args": []
    }
  }
}
```
> 默认 `--auto-connect`：如果用户 Chrome 开了远程调试 → 复用它（Google 搜索）；否则 → 自动启动 headless（Bing 搜索）

**Cursor** (`~/.cursor/mcp.json`):
```json
{
  "mcpServers": {
    "web-search": {
      "command": "ailonk-search",
      "args": []
    }
  }
}
```

**Codex / Windsurf / Cline / Continue.dev / Zed**:
```json
{
  "mcpServers": {
    "web-search": {
      "command": "ailonk-search"
    }
  }
}
```

**复用用户 Chrome（推荐，获得最佳 Google 搜索体验）:**
```bash
# 先让 Chrome 开启远程调试
# macOS:
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome --remote-debugging-port=9222

# 然后 AI 工具自动连接
```

## 9. 部署模式

### 9.1 本地使用 (Claude Code / Cursor / Codex)

AI 工具自动 spawn MCP Server 进程，通过 stdio 通信。无需手动启动任何服务。

### 9.2 服务器部署（设计目标，P3 阶段实现）

`rmcp` 1.7 支持 Streamable HTTP 传输（feature: `transport-streamable-http-server`），需配合 `axum` 构建 HTTP 服务：

```bash
# 服务器上安装 Chrome
apt install chromium-browser

# 启动 HTTP 模式
ailonk-search --transport http --port 8080 --headless --auth-token "secret"
```

**LangGraph 集成示例：**
```python
from langchain_mcp_adapters.client import MultiServerMCPClient

client = MultiServerMCPClient({
    "web-search": {
        "transport": "streamable_http",
        "url": "http://your-server:8080/mcp",
    }
})
tools = await client.get_tools()

graph = StateGraph(AgentState)
graph.add_node("search", ToolNode(tools))
```

> **注意：** HTTP 模式需完成 MCP 协议握手（initialize → tools/list → tools/call），不能直接 POST tools/call。推荐使用 MCP Inspector 或 SDK 客户端测试。

**HTTP 模式安全：**
- 默认 bind `127.0.0.1`（非 `0.0.0.0`），仅本机可访问
- `--auth-token` 启用 Bearer Token 认证
- 暴露到公网时**必须**配合反向代理 + TLS

### 9.3 Docker 部署

```dockerfile
FROM rust:1.85-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim

# Chrome + sandbox 安全配置
RUN apt-get update && \
    apt-get install -y chromium ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# 非 root 用户运行 Chrome
RUN groupadd -r chrome && useradd -r -g chrome -G audio,video chrome
USER chrome

COPY --from=builder /app/target/release/ailonk-search /usr/local/bin/

EXPOSE 8080
CMD ["ailonk-search", "--transport", "http", "--port", "8080", \
     "--headless", "--chrome-args=--disable-dev-shm-usage"]
```

**Docker 运行：**
```bash
docker run -p 8080:8080 \
  --shm-size=2g \
  ailonk-search
```

> **关键：** `--shm-size=2g` 解决 Chrome 默认 64MB `/dev/shm` 导致的渲染崩溃。`--disable-dev-shm-usage` 让 Chrome 使用 `/tmp` 替代。非 root 用户避免 `--no-sandbox` 安全风险。

### 9.4 传输模式总结

| 模式 | 启动方式 | 适用场景 | 阶段 |
|------|---------|---------|------|
| **stdio** (默认) | 被 AI 工具自动 spawn | Claude Code, Cursor, Codex | P0 |
| **Streamable HTTP** | `--transport http --port 8080` | 服务器, LangGraph | P3 |
| **Docker** | `docker run --shm-size=2g ...` | 云端/CI | P3 |

## 10. 开发阶段

| 阶段 | 内容 | 交付物 |
|------|------|--------|
| **P0: MVP** | MCP Server (stdio + rmcp) + Chrome 连接 (auto-connect) + BrowserManager (Arc\<Browser\> + handler loop) + 基础 Tab 管理 (单 Tab 串行即可) + `web_search`(Google + Bing, auto 选择) + `read_page`(dom_smoothie) + CAPTCHA 检测 + 请求限速 (jitter) | 可用的搜索+阅读工具 |
| **P1: 增强** | Tab 池并发 + `batch_read` + `search_and_read` + moka 缓存 (TTL) + 引擎降级链 | 完整核心能力 |
| **P2: 多引擎** | DuckDuckGo + 百度 (可选) + 引擎可插拔 + UA 轮换 | 灵活性 |
| **P3: HTTP 传输** | Streamable HTTP (axum + rmcp) + Docker 镜像 + auth-token | 服务器部署能力 |
| **P4: 健壮性** | 完整超时/重试 + stealth flags + 反检测增强 + 可观测性 (tracing spans) | 生产可用 |
| **P5: 发布** | README + GitHub Actions CI + 多平台二进制 (macOS/Linux/Windows) + 安装脚本 | 可分发 |
| **P6: HTTP Fallback** | 可选纯 HTTP 模式（无 Chrome 时降级，仅 DDG html + 静态页面） | 零依赖备选 |

## 11. 关键设计决策记录

| 决策 | 选择 | 理由 |
|------|------|------|
| 语言 | Rust | 官方 MCP SDK (rmcp)、成熟 CDP 库 (chromiumoxide)、单二进制分发 |
| Chrome 策略 | Chrome-first, HTTP fallback (P6) | 复用 Chrome 能力是核心价值，真实浏览器反爬更强 |
| CDP 库 | chromiumoxide 0.9 | Rust 最成熟的 async CDP 库，1300+ ⭐；`tokio` 是硬依赖无需 feature |
| MCP SDK | rmcp 1.7 (官方) | 3500+ ⭐，`#[tool]` 宏支持，Streamable HTTP 传输 |
| 正文提取 | dom_smoothie 0.18 | 最接近 Mozilla Readability.js，内置 `TextMode::Markdown` 直出 |
| HTML→MD (备用) | htmd 0.5 | 仅在 dom_smoothie HTML 模式 + 需精细转换时使用 |
| 缓存 | moka 0.12 | 替代 lru（不支持 TTL），async + TTL + 并发 |
| 默认搜索引擎 | auto (复用 Chrome → Google；headless → Bing) | 大陆用户友好，复用 Chrome 登录态时 Google 反爬风险极低 |
| 并发模型 | Tab Pool + tokio async | 每 Tab 独立 Target，无需全局锁，tokio 调度高效 |
| 搜索引擎 | 可配置 (DDG/Bing/Google) + 降级链 | 灵活应对不同地区/反爬策略 |
| 传输 | stdio (P0) + Streamable HTTP (P3) | 本地 AI 工具 + 服务器部署两种场景 |
| 部署 | 单二进制 + Docker | 简单安装，云端可用 |
| HTTP 安全 | 默认 127.0.0.1 + auth-token | 防止未授权访问 |

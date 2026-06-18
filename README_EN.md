[中文](README.md) | **English**

# ailonk-search

**Real Chrome MCP server for AI web search and Markdown content extraction.**

`ailonk-search` is a Rust-based [Model Context Protocol (MCP)](https://modelcontextprotocol.io) server that drives a real Chrome/Chromium browser via CDP. It gives AI agents reliable web search and page reading with anti-bot protection, smart page-load detection, and token-efficient Markdown output.

---

## Features

- **5 MCP tools** — `web_search`, `read_page`, `batch_read`, `search_and_read` (recommended), and `screenshot`
- **Anti-bot stealth** — powered by [eoka](https://crates.io/crates/eoka): binary patching, human-like input, and consistent fingerprints
- **Two connection modes** — **Headless** (zero-config server) and **UserChrome** (preserves login state, cookies, and extensions)
- **ML content extraction** — [rs-trafilatura](https://crates.io/crates/rs-trafilatura) extracts main content across 7 page types (article, blog, news, product, forum, documentation, generic)
- **Smart page loading** — detects SSR vs SPA pages and waits accordingly (DOM content check → network idle fallback)
- **Tab pool + caching** — semaphore-limited concurrent tabs with [moka](https://crates.io/crates/moka) content cache (default TTL 300 s)
- **Lazy Chrome init** — Chrome starts on the first tool call, not on `tools/list` (fast MCP handshake)
- **Multi-engine search** — Google, Bing, DuckDuckGo with region-aware auto-selection (`cn` → cn.bing.com)
- **Auto browser detection** — finds Google Chrome, Microsoft Edge, or Chromium on macOS, Linux, and Windows

---

## Quick Start

### Install

**Via npm (recommended, no Rust required):**

```bash
npx ailonk-search --help
# or install globally
npm install -g ailonk-search
```

**From GitHub Releases:**

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

**From source (requires Rust 1.91+):**

```bash
git clone https://github.com/lk19940215/ailonk-search.git
cd ailonk-search
cargo install --path .
```

### 2. Choose a Mode

> **Important:** Without running `setup`, the tool automatically falls back to Headless mode (no visible browser window).

| Mode | Best for | Requires setup? |
|------|----------|----------------|
| **Headless** (default fallback) | Servers, CI, quick trial, zero-config | No |
| **UserChrome** | Sites requiring login (GitHub, paywalled content) | **Yes — run `setup` first** |

**Headless mode** (zero-config, skip to step 3):

```bash
# Nothing to do — Headless works out of the box
```

**UserChrome mode** (preserves login state, cookies, extensions):

```bash
# npm users:
npx ailonk-search setup

# GitHub Releases / source build users:
ailonk-search setup
```

`setup` creates a dedicated debug Chrome profile on port 19222. It won't affect your normal browser. After setup, log in to the sites you need in that Chrome window — the login state will be used for future searches.

### 3. Configure MCP

Add `ailonk-search` to your AI tool's MCP config. The server speaks stdio MCP — no HTTP port needed.

#### Cursor

Edit `~/.cursor/mcp.json` (or project `.cursor/mcp.json`):

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

> This auto-detects: has Profile → UserChrome mode, no Profile → Headless mode.
> To force Headless: `"args": ["-y", "ailonk-search", "--headless"]`

#### Claude Code

Edit `~/.claude/settings.json` or project `.mcp.json`:

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

**Other configuration examples:**

```json
// Force Headless mode
"args": ["-y", "ailonk-search", "--headless"]

// Custom Chrome path + China region
"args": ["-y", "ailonk-search", "--chrome-path", "/path/to/chrome", "--region", "cn"]

// Allow accessing private/internal URLs
"args": ["-y", "ailonk-search", "--allow-private-urls"]
```

---

## Tools

| Tool | Description | Key parameters |
|------|-------------|----------------|
| `search_and_read` | **Recommended.** Search the web and read top results in one call. | `query`, `engine` (auto/google/bing/duckduckgo), `search_count` (1–20, default 10), `read_count` (1–5, default 3), `max_length_per_page` (default 5000) |
| `web_search` | Search only — returns titles, URLs, and snippets. | `query`, `engine`, `count` (1–20, default 10) |
| `read_page` | Fetch a single URL and extract clean Markdown. | `url`, `include_links` (default true), `max_length` (default 15000) |
| `batch_read` | Read up to 10 URLs concurrently. | `urls`, `max_length_per_page` (default 5000), `concurrency` (default 5, max 10) |
| `screenshot` | Capture a page as PNG/JPEG (base64 or file). Prefer `read_page` for text. | `url`, `format` (png/jpeg), `file_path` (optional) |

**Recommended workflow:** start with `search_and_read` → use `read_page` for specific URLs → use `web_search` when you only need result lists.

---

## Configuration

Global CLI flags (apply to all subcommands):

| Flag | Default | Description |
|------|---------|-------------|
| `--remote-url` | — | Connect to an existing Chrome CDP endpoint (e.g. `http://127.0.0.1:19222`) |
| `--headless` | false | Force headless Chrome (skip auto-connect to UserChrome) |
| `--chrome-path` | auto-detect | Path to Chrome/Edge/Chromium executable |
| `--engine` | `auto` | Default search engine: `auto`, `google`, or `bing` |
| `--region` | `auto` | Search region: `auto` (detect locale), `cn`, or `global` |
| `--max-tabs` | `5` | Maximum concurrent browser tabs |
| `--chrome-args` | — | Extra Chrome launch args (comma-separated) |
| `--cache-ttl` | `300` | Content cache TTL in seconds (`0` to disable) |
| `--allow-private-urls` | false | Allow accessing private/internal URLs (127.0.0.1, 192.168.*, etc.) |

### Subcommands

| Command | Description |
|---------|-------------|
| `ailonk-search` / `serve` | Run as MCP server (default) |
| `setup` | One-time UserChrome profile setup |
| `cleanup` | Remove debug profile symlink and print recovery steps |
| `test-all` | Run all test scenarios with a structured report |
| `test-search` | Dev: test search directly (no MCP) |
| `test-read` | Dev: test page reading directly |
| `test-search-and-read` | Dev: test search + read flow |

---

## Modes

### Headless

- Launched automatically when `--headless` is set, no setup profile exists, or UserChrome connection fails
- Uses eoka stealth: binary patching, human mouse/typing simulation
- Best for: servers, CI, headless environments, zero-config usage
- Default engine: **Bing** (more reliable against bot detection in headless mode)
- Rate limit: 5 s between search requests

### UserChrome

- Connects to (or launches) a real Chrome instance with your profile at `~/.ailonk-search-profile`
- Preserves login state, cookies, and extensions via the `setup` command
- Runs on debug port **19222** — can coexist with your normal Chrome session
- Best for: authenticated sites, CAPTCHA-prone pages, Google search in non-CN regions
- Default engine: **Google** (global) or **Bing cn** (China)
- Rate limit: 2 s between search requests

| Scenario | Recommended mode |
|----------|-----------------|
| Quick setup, server/CI | Headless (`--headless`) |
| Logged-in sites (GitHub, docs behind auth) | UserChrome (`setup` + default args) |
| China mainland search | UserChrome or `--region cn` |
| Connect to existing Chrome | `--remote-url http://127.0.0.1:9222` |

---

## Development

### Build

```bash
cargo build
cargo build --release
```

### Run locally (stdio MCP)

```bash
cargo run -- --headless          # headless mode
cargo run --                     # UserChrome (requires setup)
RUST_LOG=ailonk_search=debug cargo run -- --headless
```

### Test

```bash
# Protocol tests (fast, no Chrome)
cargo test --test mcp_integration -- --test-threads=1

# Full E2E tests (requires Chrome + network)
cargo test --test mcp_integration -- --ignored --test-threads=1

# Dev smoke tests
cargo run -- test-search "Rust async programming"
cargo run -- --headless test-read "https://example.com"
cargo run -- --headless test-search-and-read "MCP protocol" --read-count 2
cargo run -- test-all
```

### Project structure

```
src/
├── server/       # MCP tool definitions and handlers
├── browser/      # Chrome CDP manager, tab pool, navigation
├── search/       # Google, Bing, DuckDuckGo engines
├── extract/      # rs-trafilatura content extraction
├── cache/        # moka content cache
└── commands/     # CLI subcommands (serve, setup, test)
```

---

## License

MIT — See [LICENSE](LICENSE).

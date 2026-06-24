[中文](README.md) | **English**

# ailonk-search

**Real Chrome MCP server for AI web search and Markdown content extraction.**

`ailonk-search` is a Rust-based [Model Context Protocol (MCP)](https://modelcontextprotocol.io) server that drives a real Chrome/Chromium browser via CDP. It gives AI agents reliable web search and page reading with anti-bot protection, smart page-load detection, and token-efficient Markdown output.

---

## Features

- **7 MCP tools** — `search_and_read` (recommended), `web_search`, `read_page`, `batch_read`, `screenshot`, `click_authorize`, `sync_login`
- **Anti-bot stealth** — powered by [eoka](https://crates.io/crates/eoka): binary patching, human-like mouse input, and consistent fingerprints; search queries inserted via CDP insertText (~50x faster than per-character typing)
- **Google snippet extraction** — structured DOM fallback for more reliable search snippets
- **Three connection modes** — **AutoConnect** ⭐ (Chrome 144+, uses main browser with all login state), **UserChrome** (dedicated debug profile), **Headless** (zero-config)
- **ML content extraction** — [rs-trafilatura](https://crates.io/crates/rs-trafilatura) extracts main content across 7 page types (article, blog, news, product, forum, documentation, generic)
- **Content quality gating** — CAPTCHA/login-wall detection + quality scoring; low-quality pages marked `[READ_FAILED]` and not cached
- **Smart page loading** — network idle + MutationObserver-based DOM stability detection for precise SPA render completion
- **Tab pool + caching** — semaphore-limited concurrent tabs with [moka](https://crates.io/crates/moka) content cache (disabled by default, enable with `--cache-ttl`)
- **Lazy Chrome init** — Chrome starts on the first tool call, not on `tools/list` (fast MCP handshake)
- **Auto-reconnect** — detects CDP disconnection and automatically reconnects on next tool call
- **Multi-engine search** — Google, Bing, DuckDuckGo with region-aware auto-selection (`cn` → cn.bing.com)
- **Auto browser detection** — finds Google Chrome, Microsoft Edge, or Chromium on macOS, Linux, and Windows

---

## Quick Start

### 1. Install

**Via npm (recommended, no Rust required):**

```bash
# Global install (recommended)
npm install -g ailonk-search

# Or run directly via npx
npx -y ailonk-search --help
```

The npm package version is synced with GitHub Releases. The installer downloads the platform-specific binary with 3 retries, 60s timeout, and progress display. Update: `npm update -g ailonk-search`.

**Environment variables (optional):**

```bash
# China users: mirror for faster downloads and version detection
GITHUB_MIRROR=https://ghproxy.com npm install -g ailonk-search

# Pin a specific version
AILONK_VERSION=0.1.5 npm install -g ailonk-search
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

| Mode | Best for | Setup | Login state |
|------|----------|-------|-------------|
| **AutoConnect** ⭐ | Need login state (recommended, Chrome 144+) | Enable one toggle in Chrome | All main Chrome logins available instantly |
| **UserChrome** | Need login state, but don't want to modify main Chrome | Run `setup` | Manual login or `sync` |
| **Headless** (default fallback) | Servers, CI, quick trial | Zero-config | None |

**AutoConnect mode** (recommended, all login state available instantly):

1. Open `chrome://inspect/#remote-debugging` in Chrome and enable "Enable remote debugging" (one-time)
2. Done! ailonk-search will auto-connect to your main Chrome

> Chrome will show a permission dialog on first connection — click "Allow".

**UserChrome mode** (dedicated debug profile):

```bash
# npm users:
npx ailonk-search setup

# GitHub Releases / source build users:
ailonk-search setup
```

`setup` creates a dedicated debug Chrome profile on port 19222. It won't affect your normal browser and **works with MCP integration**. When login state expires, run `ailonk-search sync` or use the MCP tool `sync_login` to refresh; however, `sync_login` **cannot** transfer Google OAuth sessions (Chrome encrypts OAuth cookies), so Google auth must be completed manually once in the debug profile, or use AutoConnect mode instead.

**Headless mode** (zero-config, skip to step 3):

```bash
# Nothing to do — Headless works out of the box
```

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

> Auto-detects: AutoConnect → UserChrome → Headless.
> To force Headless: `"args": ["-y", "ailonk-search", "--headless"]`

<details>
<summary>Global install method (requires full path)</summary>

```bash
npm install -g ailonk-search
```

Cursor does not inherit shell environment variables, so you must use the full binary path:

```json
{
  "mcpServers": {
    "ailonk-search": {
      "command": "/path/to/node/bin/ailonk-search",
      "args": ["serve"]
    }
  }
}
```

Find your path: `which ailonk-search`

</details>

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
| `search_and_read` | **Recommended.** Search and read top results in one call. | `query`, `engine` (auto/google/bing/duckduckgo), `search_count` (1–20, default 10), `read_count` (1–5, default 3), `max_length_per_page` (default 5000) |
| `web_search` | Search only — returns titles, URLs, and snippets. | `query`, `engine`, `count` (1–20, default 10) |
| `read_page` | Fetch a single URL and extract clean Markdown. | `url`, `include_links` (default true), `max_length` (default 15000) |
| `batch_read` | Read up to 10 URLs concurrently. | `urls`, `max_length_per_page` (default 5000), `concurrency` (default 5, max 10) |
| `screenshot` | Capture a page as PNG/JPEG (base64 or file). Prefer `read_page` for text. | `url`, `format` (png/jpeg), `file_path` (optional) |
| `click_authorize` | Detect and click OAuth/SSO authorization pages (SSO buttons, consent pages, SAML, popups, multi-step redirects) | `url`, `timeout` (default: 30s) |
| `sync_login` | Sync login state from main Chrome to debug profile (UserChrome only; cannot sync Google OAuth sessions) | No parameters |

**Recommended workflow:** `search_and_read` → `read_page` (specific URLs) → `web_search` (only need result lists)

**Auth failure handling:** When `read_page` returns `[READ_FAILED]` → try `click_authorize` first (OAuth/SSO pages) → if still failing due to expired cookies/sessions, call `sync_login` in UserChrome mode (Google OAuth sessions cannot be synced — manual auth or AutoConnect required)

### click_authorize — OAuth/SSO Authorization

Detects and clicks through authorization pages to complete OAuth/SSO login flows. Use when `read_page` returns `[READ_FAILED]` because the page requires authorization.

**Capabilities:**

- Click SSO/login buttons (e.g. "AKULAKU SSO Login", "Sign in with Google")
- Handle OAuth/SSO popups
- Manage multi-step SSO redirect chains
- Handle Google SAML SSO pages
- Web-based Google account selection (`accounts.google.com` page redirects)
- Returning users: Chrome FedCM auto-reauthn may complete authentication after the tool triggers the flow

**Limitations (CDP architecture):**

- **Cannot** interact with the FedCM browser-native popup (Chrome top-level UI account picker — not DOM, not accessible via CDP)
- For sites using FedCM for the first time: user must manually complete Google authorization once, after which auto-reauthn handles subsequent authorizations

**Not for:** Expired cookies/sessions (use `sync_login` in UserChrome mode, except Google OAuth), username/password login, CAPTCHA, or multi-factor authentication

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `url` | string | required | URL that requires authorization |
| `timeout` | uint | 30 | Max seconds to wait for the auth flow to complete |

**Flow:** Navigate → detect auth type → click button → wait for Chrome to handle natively → redirect

**Example:**

```json
{
  "tool": "click_authorize",
  "arguments": {
    "url": "https://docs.google.com/document/d/abc123/edit",
    "timeout": 30
  }
}
```

**Typical workflow:**

```
1. read_page("https://docs.google.com/...")  →  [READ_FAILED] (OAuth blocked)
2. click_authorize("https://docs.google.com/...")  →  authorization succeeded
3. read_page("https://docs.google.com/...")  →  returns Markdown content
```

---

## Configuration

Global CLI flags (apply to all subcommands):

| Flag | Default | Description |
|------|---------|-------------|
| `--remote-url` | — | Connect to an existing Chrome CDP endpoint |
| `--headless` | false | Force headless Chrome |
| `--chrome-path` | auto-detect | Path to Chrome/Edge/Chromium executable |
| `--engine` | `auto` | Default search engine: `auto`, `google`, `bing` |
| `--region` | `auto` | Search region: `auto` (detect locale), `cn`, `global` |
| `--max-tabs` | `5` | Maximum concurrent browser tabs |
| `--chrome-args` | — | Extra Chrome launch args (comma-separated) |
| `--cache-ttl` | `0` | Content cache TTL in seconds (`0` to disable, e.g. `300` to enable) |
| `--allow-private-urls` | false | Allow accessing private/internal URLs (127.0.0.1, 192.168.*, etc.) |

### Subcommands

| Command | Description |
|---------|-------------|
| `ailonk-search` / `serve` | Run as MCP server (default) |
| `setup` | One-time UserChrome profile setup |
| `sync` | Sync login state (Cookies + localStorage + sessionStorage) from main Chrome |
| `cleanup` | Remove debug profile and print recovery steps |
| `test-all` | Run all test scenarios with a structured report |
| `test-search` | Dev: test search directly (no MCP) |
| `test-read` | Dev: test page reading directly |
| `test-search-and-read` | Dev: test search + read flow |

---

## Modes

### AutoConnect ⭐ (Chrome 144+)

- Auto-detects Chrome's `DevToolsActivePort` and connects via WebSocket
- **All login state available instantly** — no sync, no separate profile
- One-time setup: enable `chrome://inspect/#remote-debugging` in Chrome
- Chrome shows a permission dialog on first connection (click "Allow")
- Default engine: **Google** (global) or **Bing cn** (China)
- Rate limit: 2 s between search requests

### UserChrome

- Connects to (or launches) a real Chrome instance with profile at `~/.ailonk-search-profile`
- **MCP integration supported** (same config as AutoConnect/Headless)
- Sync login state via `sync` command or MCP `sync_login` tool (cookies, localStorage, etc.)
- `sync_login` **cannot** sync Google OAuth sessions (Chrome cookie encryption); Google services require manual first-time auth in the debug profile, or use AutoConnect
- Runs on debug port **19222** — coexists with normal Chrome
- Default engine: **Google** (global) or **Bing cn** (China)
- Rate limit: 2 s between search requests

### Headless

- Activated when `--headless` is set, or no UserChrome profile exists, or AutoConnect unavailable
- Uses eoka stealth: binary patching, human mouse/typing simulation
- Best for: servers, CI, headless environments, zero-config usage
- Default engine: **Bing** (more reliable in headless mode)
- Rate limit: 5 s between search requests

### Connection Priority

ailonk-search tries connections in this order:

1. `--remote-url` (explicit)
2. `DevToolsActivePort` (AutoConnect, Chrome 144+)
3. Port 19222 (UserChrome, created by `setup`)
4. Auto-launch Chrome (UserChrome or Headless fallback)

| Scenario | Recommended mode |
|----------|-----------------|
| Sites requiring login | **AutoConnect** (enable `chrome://inspect`) |
| Don't want to modify main Chrome | UserChrome (run `setup` first) |
| Server / CI / quick trial | Headless (`--headless`) |
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
cargo run -- --headless                    # Headless mode
cargo run --                               # UserChrome mode (requires setup)
RUST_LOG=ailonk_search=debug cargo run -- --headless  # Debug logs
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
├── browser/      # Chrome CDP manager, tab pool, page interaction
├── search/       # Google, Bing, DuckDuckGo engines
├── extract/      # rs-trafilatura content extraction
├── cache.rs      # moka content cache
└── commands/     # CLI subcommands (serve, setup, sync, cleanup, test)
```

---

## License

MIT — See [LICENSE](LICENSE).

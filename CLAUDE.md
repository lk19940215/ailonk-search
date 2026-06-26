# CLAUDE.md — ailonk-search Project Guide

## Overview

ailonk-search is a Rust MCP server providing AI agents with web search and page reading via real Chrome browser (CDP). Built on eoka (stealth CDP) + rmcp (MCP SDK).

## Architecture

```
AI Tool → stdio/JSON-RPC → ailonk-search (Rust) → eoka CDP → Chrome Browser
```

Three layers:
1. **MCP Server** (`src/server/`) — Tool handlers, timeout management, browser acquisition
2. **Browser Abstraction** (`src/browser/`) — Chrome lifecycle, tab pool, page interaction
3. **Domain Logic** (`src/search/`, `src/extract/`, `src/cache.rs`) — Search engines, content extraction, caching

## Source Structure

```
src/
├── main.rs                    # Entry point: CLI parsing + MCP server startup
├── cli.rs                     # CLI argument definitions (clap)
├── cache.rs                   # ContentCache (moka TTL cache)
├── server/
│   ├── mod.rs                 # MCP tool router, browser acquisition, timeout wrappers,
│   │                          # shared helpers (acquire_tab, navigate_and_fetch_html,
│   │                          # fetch_and_extract, read_urls_concurrent, with_hard_timeout)
│   ├── tools.rs               # Tool parameter structs (schemars JSON Schema)
│   ├── authorize.rs           # click_authorize handler: SSO loop, redirect/popup race
│   └── popup_handler.rs       # handle_popup handler: generic popup lifecycle
├── browser/
│   ├── mod.rs                 # Re-exports
│   ├── manager.rs             # LazyBrowserManager (reconnect, spawn_blocking, cached WS URL),
│   │                          # BrowserManager (connect methods, health check, shutdown)
│   ├── pool.rs                # TabPool + TabGuard (semaphore-limited concurrent tabs)
│   ├── profile.rs             # Debug Chrome profile management, login state sync
│   └── interaction/
│       ├── mod.rs             # Re-exports for navigate, click, captcha, consent, validation
│       ├── auth.rs            # Auth page detection (AuthPageType), click strategies,
│       │                      # Google account selection, SSO button scoring
│       ├── popup_flow.rs      # Shared popup logic: prepare, race redirect vs popup,
│       │                      # interact_with_auth_page, handle_auth_popup
│       ├── target_watcher.rs  # Tab lifecycle monitor (poll-based new tab detection)
│       ├── signals.rs         # Auth URL patterns, SSO signals, button keywords
│       ├── click.rs           # Generic click utilities (by text, selector, coordinates)
│       ├── captcha.rs         # CAPTCHA detection and resolution
│       ├── consent.rs         # Cookie consent banner handling
│       ├── input.rs           # Safe JS evaluation helpers
│       └── navigate.rs        # URL navigation with smart waiting
├── search/
│   ├── mod.rs                 # Module re-exports
│   ├── engine.rs              # SearchEngine trait, engine selection, fallback chain, rate limiter
│   ├── google.rs              # Google SERP parsing
│   ├── bing.rs                # Bing SERP parsing
│   └── duckduckgo.rs          # DuckDuckGo SERP parsing
├── extract/
│   ├── mod.rs                 # Re-exports content module
│   └── content.rs             # ContentExtractor (rs-trafilatura), quality scoring, markdown links
└── commands/
    ├── mod.rs                 # Subcommand dispatch
    ├── serve.rs               # MCP server startup (default subcommand)
    ├── setup.rs               # `setup` subcommand (create debug profile)
    ├── sync.rs                # `sync` subcommand (sync login state)
    ├── cleanup.rs             # `cleanup` subcommand (remove debug profile)
    └── test.rs                # `test-*` subcommands (dev testing)
```

## Key Architectural Patterns

### Timeout Defense
- **Tool level**: `with_hard_timeout` (spawn-isolated oneshot channel) — guarantees MCP response even if CDP blocks
- **Connection level**: `spawn_connect` uses `spawn_blocking` to keep eoka's blocking I/O off async worker threads
- **Page level**: `FETCH_HARD_TIMEOUT` (45s) wraps navigate+extract; `TAB_ACQUIRE_TIMEOUT` (10s) for tab pool

### Browser Connection
- `LazyBrowserManager::get()` — cached healthy connection (fast path: read lock) → reconnect (slow path: reconnect_mutex serialization)
- `spawn_blocking` for eoka connections — prevents tokio thread starvation
- Connection priority (non-headless): cached WS URL (reconnect) → debug port 19222 (if profile exists) → DevToolsActivePort (AutoConnect) → launch UserChrome; also supports `--remote-url` and `--headless`

### Auth Flow
- `click_authorize` → SSO loop: detect auth type → try_sso_click → race redirect vs popup → handle popup
- `handle_popup` → navigate + trigger + wait for popup → attach + detect + interact → wait for close
- Shared logic in `popup_flow.rs`: `interact_with_auth_page`, `handle_auth_popup`, `race_redirect_vs_popup`

## 8 MCP Tools

1. `web_search` — Search via real Chrome (Google/Bing/DDG)
2. `read_page` — Fetch URL + extract Markdown content
3. `batch_read` — Concurrent multi-URL reading
4. `search_and_read` — Search + auto-read top results (recommended)
5. `screenshot` — Page screenshot (base64 or file)
6. `click_authorize` — OAuth/SSO authorization flow handler
7. `handle_popup` — General popup/new-tab handler (auth + non-auth + observe)
8. `sync_login` — Sync login state from main Chrome (UserChrome mode only)

## Development

```bash
cargo build                  # Debug build
cargo build --release        # Release build
cargo test --test mcp_integration -- --test-threads=1  # Protocol tests
RUST_LOG=ailonk_search=debug cargo run  # Debug logging
```

## Conventions

- Error types: `ErrorData` (MCP layer via `to_mcp_error`), `anyhow::Error` (internal)
- Logging: `tracing` crate; `info!` for user-significant events, `debug!` for internal flow
- PII: No emails at info level; account details only at debug
- Timeouts: All CDP operations bounded; `spawn_blocking` for eoka blocking I/O
- Tab lifecycle: Always `tab.close().await` after use (in TabGuard drop or explicit)

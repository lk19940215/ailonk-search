//! MCP integration tests for ailonk-search.
//!
//! These tests simulate how AI tools (Codex, Claude Code, Cursor) interact
//! with the MCP server: discover tools → call tools → use results to call
//! more tools within the same session.
//!
//! Run protocol-only tests (fast, no Chrome):
//!   cargo test --test mcp_integration -- --test-threads=1
//!
//! Run full E2E tests (requires Chrome + network):
//!   cargo test --test mcp_integration -- --ignored --test-threads=1

use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};

type McpClient = rmcp::service::RunningService<rmcp::RoleClient, ()>;

const EXPECTED_TOOLS: &[&str] = &[
    "web_search",
    "read_page",
    "batch_read",
    "search_and_read",
    "screenshot",
    "click_authorize",
    "sync_login",
];

async fn spawn_client(extra_args: &[&str]) -> anyhow::Result<McpClient> {
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_ailonk-search")).configure(|cmd| {
            cmd.arg("--headless").arg("--cache-ttl").arg("300");
            for arg in extra_args {
                cmd.arg(arg);
            }
        }),
    )?;
    Ok(().serve(transport).await?)
}

fn call(name: &str, args: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_string()).with_arguments(
        args.as_object().expect("args must be JSON object").clone(),
    )
}

fn extract_text(result: &rmcp::model::CallToolResult) -> Option<&str> {
    result
        .content
        .first()
        .and_then(|c| c.raw.as_text())
        .map(|t| t.text.as_str())
}

fn extract_urls_from_search_results(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            if let Some(start) = line.find("](http") {
                let url_start = start + 2;
                if let Some(end) = line[url_start..].find(')') {
                    return Some(line[url_start..url_start + end].to_string());
                }
            }
            None
        })
        .collect()
}

// ============================================================
// Test 1: Protocol handshake + tool discovery (no Chrome needed)
// ============================================================

#[tokio::test]
async fn t1_initialize_returns_server_info() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;
    let info = client.peer_info().expect("Server should return peer info");

    assert_eq!(
        info.server_info.name.as_str(),
        "ailonk-search",
        "Server name mismatch"
    );
    assert!(
        info.instructions.as_deref().unwrap_or("").contains("search_and_read"),
        "Instructions should mention search_and_read tool"
    );
    assert!(
        info.capabilities.tools.is_some(),
        "Server should declare tool capabilities"
    );

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn t1_tools_list_complete() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;
    let tools = client.list_all_tools().await?;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    for expected in EXPECTED_TOOLS {
        assert!(names.contains(expected), "Missing tool: {expected}");
    }
    assert_eq!(
        tools.len(),
        EXPECTED_TOOLS.len(),
        "Extra tools found: {names:?}"
    );

    for tool in &tools {
        assert!(
            !tool.description.as_deref().unwrap_or("").is_empty(),
            "Tool {} lacks description — AI cannot discover it",
            tool.name
        );
    }

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn t1_web_search_schema_has_required_query() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;
    let tools = client.list_all_tools().await?;

    let ws = tools
        .iter()
        .find(|t| t.name.as_ref() == "web_search")
        .expect("web_search not found");
    let schema = serde_json::to_string(&ws.input_schema)?;

    assert!(schema.contains("query"), "web_search schema must have 'query' field");
    assert!(schema.contains("required"), "web_search must declare required fields");

    client.cancel().await?;
    Ok(())
}

// ============================================================
// Test 2: Error handling (no Chrome for param validation)
// ============================================================

#[tokio::test]
async fn t2_web_search_missing_query_returns_error() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    let result = client.call_tool(call("web_search", serde_json::json!({}))).await;

    assert!(
        result.is_err() || result.as_ref().unwrap().is_error == Some(true),
        "web_search without required 'query' should fail"
    );

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn t2_read_page_invalid_url_returns_error() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    let result = client
        .call_tool(call("read_page", serde_json::json!({ "url": "not-a-url" })))
        .await;

    assert!(
        result.is_err() || result.as_ref().unwrap().is_error == Some(true),
        "read_page with invalid URL should fail"
    );

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn t2_screenshot_invalid_url_returns_error() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    let result = client
        .call_tool(call("screenshot", serde_json::json!({ "url": "not-a-url" })))
        .await;

    assert!(
        result.is_err() || result.as_ref().unwrap().is_error == Some(true),
        "screenshot with invalid URL should fail"
    );

    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn t2_batch_read_empty_urls_returns_message() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    let result = client
        .call_tool(call("batch_read", serde_json::json!({ "urls": [] })))
        .await?;

    let text = extract_text(&result).unwrap_or("");
    assert!(
        text.contains("No URLs") || result.is_error == Some(true),
        "batch_read with empty URLs should return a message, got: {text}"
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================
// Test 3: Multi-turn AI workflow (requires Chrome + network)
//
// Simulates: AI discovers tools → search → parse results →
// read a specific page → verify cache hit on second read.
// All within ONE session.
// ============================================================

#[tokio::test]
#[ignore = "requires Chrome and network"]
async fn t3_multi_turn_search_then_read() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    // Step 1: AI discovers tools (just like Codex/Cursor does on connect)
    let tools = client.list_all_tools().await?;
    let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(tool_names.contains(&"web_search"));
    assert!(tool_names.contains(&"read_page"));

    // Step 2: AI calls web_search (simulates AI deciding to search)
    let search_result = tokio::time::timeout(
        std::time::Duration::from_secs(90),
        client.call_tool(call(
            "web_search",
            serde_json::json!({
                "query": "Rust programming language official",
                "engine": "bing",
                "count": 5
            }),
        )),
    )
    .await??;

    assert_ne!(search_result.is_error, Some(true), "web_search failed");
    let search_text = extract_text(&search_result).expect("search should return text");

    assert!(
        search_text.contains("results for"),
        "search output should match 'Found N results for' format"
    );
    let urls = extract_urls_from_search_results(search_text);
    assert!(
        !urls.is_empty(),
        "search results should contain at least one URL, got:\n{search_text}"
    );

    // Step 3: AI picks a URL from results and calls read_page
    // (simulates AI analyzing search results and deciding to read more)
    let target_url = urls
        .iter()
        .find(|u| u.contains("rust-lang") || u.contains("rust"))
        .unwrap_or(&urls[0]);

    let read_start = std::time::Instant::now();
    let read_result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.call_tool(call(
            "read_page",
            serde_json::json!({
                "url": target_url,
                "max_length": 5000
            }),
        )),
    )
    .await??;

    assert_ne!(read_result.is_error, Some(true), "read_page failed for {target_url}");
    let read_text = extract_text(&read_result).expect("read_page should return text");
    let first_read_ms = read_start.elapsed().as_millis();

    assert!(
        read_text.len() > 100,
        "read_page should return substantial content, got {} chars",
        read_text.len()
    );

    // Step 4: AI calls read_page again for the same URL (should hit cache)
    let cache_start = std::time::Instant::now();
    let cache_result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.call_tool(call(
            "read_page",
            serde_json::json!({
                "url": target_url,
                "max_length": 5000
            }),
        )),
    )
    .await??;

    let cache_text = extract_text(&cache_result).expect("cached read should return text");
    let cache_ms = cache_start.elapsed().as_millis();

    assert!(
        cache_text.contains("cached"),
        "Second read should indicate cache hit"
    );
    assert!(
        cache_ms < first_read_ms || cache_ms < 1000,
        "Cached read ({cache_ms}ms) should be faster than first ({first_read_ms}ms)"
    );

    // Verify cached content preserves the same body
    let first_body = read_text.split("---").last().unwrap_or("").trim();
    let cached_body = cache_text.split("---").last().unwrap_or("").trim();
    assert_eq!(
        first_body, cached_body,
        "Cached content should be identical to first read"
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================
// Test 4: search_and_read one-shot (requires Chrome + network)
// ============================================================

#[tokio::test]
#[ignore = "requires Chrome and network"]
async fn t4_search_and_read_returns_combined_results() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        client.call_tool(call(
            "search_and_read",
            serde_json::json!({
                "query": "Rust tokio async runtime",
                "engine": "bing",
                "search_count": 5,
                "read_count": 1,
                "max_length_per_page": 3000
            }),
        )),
    )
    .await??;

    assert_ne!(result.is_error, Some(true), "search_and_read failed");
    let text = extract_text(&result).expect("should return text");

    assert!(
        text.contains("Search results") || text.contains("search results"),
        "Output should contain search results section"
    );
    assert!(
        text.contains("Full content") || text.contains("⭐ read"),
        "Output should contain read content or read markers"
    );
    assert!(
        text.len() > 500,
        "Combined output should be substantial ({} chars)",
        text.len()
    );

    client.cancel().await?;
    Ok(())
}

// ============================================================
// Test 5: read_page standalone (requires Chrome + network)
// ============================================================

#[tokio::test]
#[ignore = "requires Chrome and network"]
async fn t5_read_page_extracts_structured_content() -> anyhow::Result<()> {
    let client = spawn_client(&[]).await?;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        client.call_tool(call(
            "read_page",
            serde_json::json!({
                "url": "https://www.rust-lang.org/",
                "max_length": 5000
            }),
        )),
    )
    .await??;

    assert_ne!(result.is_error, Some(true), "read_page failed");
    let text = extract_text(&result).expect("should return text");

    assert!(text.starts_with("# "), "Output should start with Markdown title");
    assert!(text.contains("Source:"), "Output should contain source URL");
    assert!(text.contains("---"), "Output should contain separator");

    client.cancel().await?;
    Ok(())
}

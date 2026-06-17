use std::sync::Arc;

use rmcp::ServiceExt;

pub async fn run(args: &crate::cli::Args) -> anyhow::Result<()> {
    tracing::info!("MCP Server mode");

    let region = crate::search::engine::detect_region(&args.region);
    let browser_manager = crate::browser::manager::BrowserManager::new(args).await?;
    let browser_manager = Arc::new(browser_manager);

    let server = crate::server::SearchServer::new(
        browser_manager.clone(),
        args.engine.clone(),
        args.cache_ttl,
        region,
    );

    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let running_server = server.serve(transport).await?;

    running_server.waiting().await?;
    browser_manager.shutdown().await;

    Ok(())
}

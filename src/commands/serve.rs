use rmcp::ServiceExt;

use crate::browser::manager::LazyBrowserManager;

pub async fn run(args: &crate::cli::Args) -> anyhow::Result<()> {
    tracing::info!("MCP Server mode (Chrome will start on first tool call)");

    let region = crate::search::engine::detect_region(&args.region);
    let lazy_browser = LazyBrowserManager::new(args);
    let cleanup_handle = lazy_browser.clone();

    let result = async {
        let server = crate::server::SearchServer::new(
            lazy_browser,
            args.engine.clone(),
            args.cache_ttl,
            region,
            args.allow_private_urls,
        );

        let transport = (tokio::io::stdin(), tokio::io::stdout());
        let running_server = server.serve(transport).await?;

        #[cfg(unix)]
        {
            let mut sigterm = tokio::signal::unix::signal(
                tokio::signal::unix::SignalKind::terminate(),
            )?;
            tokio::select! {
                res = running_server.waiting() => { res?; }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received SIGINT, shutting down...");
                }
                _ = sigterm.recv() => {
                    tracing::info!("Received SIGTERM, shutting down...");
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::select! {
                res = running_server.waiting() => { res?; }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received SIGINT, shutting down...");
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    }
    .await;

    cleanup_handle.shutdown().await;
    result
}

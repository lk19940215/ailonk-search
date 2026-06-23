use std::sync::Arc;

struct TestScenario {
    name: &'static str,
    kind: TestKind,
}

enum TestKind {
    Search {
        query: &'static str,
        engine: &'static str,
    },
    Read {
        url: &'static str,
    },
}

pub async fn run_all(args: &crate::cli::Args) -> anyhow::Result<()> {
    let browser_manager = crate::browser::manager::BrowserManager::new(args).await?;
    let browser_manager = Arc::new(browser_manager);

    let result = async {
        let region = crate::search::engine::detect_region(&args.region);

        println!("╔══════════════════════════════════════════════════╗");
        println!("║        Chrome Search MCP — Test Report          ║");
        println!("╠══════════════════════════════════════════════════╣");
        println!(
            "║ Mode: {:43}║",
            match browser_manager.mode() {
                crate::browser::manager::ConnectionMode::UserChrome => "UserChrome (CDP)",
                crate::browser::manager::ConnectionMode::Headless => "Headless (eoka stealth)",
            }
        );
        println!("║ Region: {:41}║", region);
        println!("╚══════════════════════════════════════════════════╝");
        println!();

        let scenarios = vec![
            TestScenario {
                name: "Search (auto engine)",
                kind: TestKind::Search {
                    query: "Rust programming language",
                    engine: "auto",
                },
            },
            TestScenario {
                name: "Search (bing)",
                kind: TestKind::Search {
                    query: "Rust async await tutorial",
                    engine: "bing",
                },
            },
            TestScenario {
                name: "Read page",
                kind: TestKind::Read {
                    url: "https://www.rust-lang.org/",
                },
            },
        ];

        let rate_limiter = crate::search::engine::RateLimiter::new(2000);
        let mut any_failed = false;

        for scenario in &scenarios {
            let start = std::time::Instant::now();
            match &scenario.kind {
                TestKind::Search { query, engine } => {
                    rate_limiter.wait().await;
                    let engine_choice = crate::server::tools::EngineChoice::from_str(engine);
                    let search_engine = crate::search::engine::select_engine(
                        &engine_choice,
                        browser_manager.mode(),
                        &args.engine,
                        region,
                    );
                    let tab = browser_manager.tab_pool().acquire().await?;

                    let result = crate::search::engine::search_with_fallback(
                        search_engine,
                        browser_manager.mode(),
                        tab.page(),
                        query,
                        5,
                        region,
                    ).await;
                    tab.close().await;
                    let elapsed = start.elapsed();

                    match result {
                        Ok((engine_name, results)) => {
                            println!(
                                "\n  [{} via {} — {} results, {:.1}s]",
                                scenario.name, engine_name, results.len(), elapsed.as_secs_f64()
                            );
                            for (i, r) in results.iter().enumerate() {
                                let snippet: String = r.snippet.chars().take(80).collect();
                                println!("  {}. [{}]({})", i + 1, r.title, r.url);
                                if !snippet.is_empty() {
                                    println!("     {}", snippet);
                                }
                            }
                            println!();
                        }
                        Err(e) => {
                            any_failed = true;
                            println!("\n  [{} — {:.1}s] ERROR: {}\n", scenario.name, elapsed.as_secs_f64(), e);
                        }
                    }
                }
                TestKind::Read { url } => {
                    use crate::browser::interaction;
                    let tab = browser_manager.tab_pool().acquire().await?;
                    let result = async {
                        interaction::navigate(tab.page(), url, 15).await?;
                        tab.page().content().await
                            .map_err(|e| anyhow::anyhow!("Content fetch failed: {}", e))
                    }.await;
                    tab.close().await;
                    let elapsed = start.elapsed();

                    match result {
                        Ok(html) => {
                            match crate::extract::content::ContentExtractor::extract(&html, url, 5000) {
                                Ok(ex) => {
                                    println!(
                                        "\n  [{} — {} chars, {:.1}s]",
                                        scenario.name, ex.content.len(), elapsed.as_secs_f64()
                                    );
                                    println!("  Title: {}", ex.title);
                                    let preview: String = ex.content.chars().take(200).collect();
                                    println!("  Preview: {}...\n", preview);
                                }
                                Err(e) => {
                                    any_failed = true;
                                    println!("\n  [{} — {:.1}s] ERROR: {}\n", scenario.name, elapsed.as_secs_f64(), e);
                                }
                            }
                        }
                        Err(e) => {
                            any_failed = true;
                            println!("\n  [{} — {:.1}s] ERROR: {}\n", scenario.name, elapsed.as_secs_f64(), e);
                        }
                    }
                }
            }
        }

        println!("\nDone.");
        if any_failed {
            anyhow::bail!("One or more test scenarios failed");
        }
        Ok(())
    }.await;

    browser_manager.shutdown().await;
    result
}

pub async fn run_search(
    args: &crate::cli::Args,
    query: &str,
    engine: &str,
    count: usize,
) -> anyhow::Result<()> {
    let browser_manager = crate::browser::manager::BrowserManager::new(args).await?;
    let browser_manager = Arc::new(browser_manager);

    let result = async {
        let region = crate::search::engine::detect_region(&args.region);
        let engine_choice = crate::server::tools::EngineChoice::from_str(engine);

        let rate_limiter = crate::search::engine::RateLimiter::new(2000);
        rate_limiter.wait().await;

        let search_engine = crate::search::engine::select_engine(
            &engine_choice,
            browser_manager.mode(),
            &args.engine,
            region,
        );
        let tab = browser_manager.tab_pool().acquire().await?;

        let result = crate::search::engine::search_with_fallback(
            search_engine,
            browser_manager.mode(),
            tab.page(),
            query,
            count,
            region,
        ).await;
        tab.close().await;

        let (engine_name, results) = result?;
        let text = crate::search::engine::format_search_results(query, &engine_name, &results);
        println!("{}", text);
        Ok(())
    }.await;

    browser_manager.shutdown().await;
    result
}

pub async fn run_search_and_read(
    args: &crate::cli::Args,
    query: &str,
    read_count: usize,
    max_length: usize,
) -> anyhow::Result<()> {
    let browser_manager = crate::browser::manager::BrowserManager::new(args).await?;
    let browser_manager = Arc::new(browser_manager);

    let result = async {
        let region = crate::search::engine::detect_region(&args.region);

        let rate_limiter = crate::search::engine::RateLimiter::new(2000);
        rate_limiter.wait().await;

        let engine = crate::search::engine::select_engine(
            &crate::server::tools::EngineChoice::Auto,
            browser_manager.mode(),
            &args.engine,
            region,
        );

        println!("Searching: \"{}\"", query);
        let start = std::time::Instant::now();
        let tab = browser_manager.tab_pool().acquire().await?;

        let search_result = crate::search::engine::search_with_fallback(
            engine,
            browser_manager.mode(),
            tab.page(),
            query,
            10,
            region,
        ).await;
        tab.close().await;

        let (engine_name, results) = search_result?;
        let search_elapsed = start.elapsed();
        println!(
            "   Found {} results via {} ({:.1}s)\n",
            results.len(), engine_name, search_elapsed.as_secs_f64()
        );

        for (i, r) in results.iter().enumerate() {
            let marker = if i < read_count { "*" } else { " " };
            println!("  {} {}. [{}]({})", marker, i + 1, r.title, r.url);
            if !r.snippet.is_empty() {
                let snippet: String = r.snippet.chars().take(80).collect();
                println!("       {}", snippet);
            }
        }
        println!();

        let urls_to_read: Vec<(usize, String, String)> = results
            .iter()
            .take(read_count)
            .enumerate()
            .map(|(i, r)| (i + 1, r.url.clone(), r.title.clone()))
            .collect();

        println!("Reading top {} pages...\n", urls_to_read.len());

        use crate::browser::interaction;
        for (idx, url, title) in &urls_to_read {
            let read_start = std::time::Instant::now();
            let tab = browser_manager.tab_pool().acquire().await?;
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                async {
                    let r = async {
                        interaction::navigate(tab.page(), url, 15).await?;
                        tab.page().content().await
                            .map_err(|e| anyhow::anyhow!("Content fetch: {}", e))
                    }.await;
                    tab.close().await;
                    r
                },
            ).await;
            let read_elapsed = read_start.elapsed();
            let result = match result {
                Ok(r) => r,
                Err(_) => Err(anyhow::anyhow!("Page read timeout after 30s")),
            };

            match result {
                Ok(html) if !html.is_empty() => {
                    match crate::extract::content::ContentExtractor::extract(&html, url, max_length) {
                        Ok(ex) => {
                            let preview: String = ex.content.chars().take(300).collect();
                            println!(
                                "  [{}] {} ({} chars, {:.1}s)",
                                idx, title, ex.content.len(), read_elapsed.as_secs_f64()
                            );
                            println!("      {}", preview);
                            println!();
                        }
                        Err(e) => {
                            println!("  [{}] {} — extract error: {} ({:.1}s)", idx, title, e, read_elapsed.as_secs_f64());
                        }
                    }
                }
                Ok(_) => {
                    println!("  [{}] {} — empty content ({:.1}s)", idx, title, read_elapsed.as_secs_f64());
                }
                Err(e) => {
                    println!("  [{}] {} — {} ({:.1}s)", idx, title, e, read_elapsed.as_secs_f64());
                }
            }
        }

        let total = start.elapsed();
        println!("Done — search + read {} pages in {:.1}s", urls_to_read.len(), total.as_secs_f64());
        Ok(())
    }.await;

    browser_manager.shutdown().await;
    result
}

pub async fn run_read(
    args: &crate::cli::Args,
    url: &str,
    max_length: usize,
) -> anyhow::Result<()> {
    let browser_manager = crate::browser::manager::BrowserManager::new(args).await?;
    let browser_manager = Arc::new(browser_manager);

    let result = async {
        let tab = browser_manager.tab_pool().acquire().await?;

        use crate::browser::interaction;
        let result = async {
            interaction::navigate(tab.page(), url, 15).await?;
            tab.page().content().await
                .map_err(|e| anyhow::anyhow!("Content fetch: {}", e))
        }.await;
        tab.close().await;

        let html = result?;
        if html.is_empty() {
            println!("Failed to load content from {}", url);
            return Ok(());
        }

        let extracted = crate::extract::content::ContentExtractor::extract(&html, url, max_length)?;
        println!("# {}\n\nSource: {}\n\n---\n\n{}", extracted.title, url, extracted.content);
        Ok(())
    }.await;

    browser_manager.shutdown().await;
    result
}

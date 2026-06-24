use eoka::Page;

pub async fn handle_consent(page: &Page, engine: &str) -> anyhow::Result<()> {
    let url = page.url().await.unwrap_or_default();

    if url.contains("consent.google.com") {
        page.try_click("button[aria-label*='Accept']").await.ok();
        page.try_click("form[action*='consent'] button").await.ok();
        page.wait(1500).await;
    }

    if engine == "bing" {
        page.try_click("#bnp_btn_accept").await.ok();
        page.try_click(".bnp_btn_accept").await.ok();
    }

    Ok(())
}

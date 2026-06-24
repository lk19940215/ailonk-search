use eoka::Page;

pub async fn extract<T: serde::de::DeserializeOwned>(
    page: &Page,
    js: &str,
) -> anyhow::Result<T> {
    page.evaluate(js).await
        .map_err(|e| anyhow::anyhow!("JS evaluation failed: {}", e))
}

/// Fill search box and submit via CDP `Input.insertText` (single-shot, no per-char delay).
/// Returns Ok(true) on success, Ok(false) if input element not found.
pub async fn type_and_submit(
    page: &Page,
    selectors: &[&str],
    text: &str,
    timeout_ms: u64,
) -> anyhow::Result<bool> {
    for sel in selectors {
        match page.wait_for(sel, timeout_ms).await {
            Ok(_) => {
                page.fill(sel, text).await
                    .map_err(|e| anyhow::anyhow!("fill failed on {}: {}", sel, e))?;
                page.wait(200).await;
                page.press_key("Enter").await
                    .map_err(|e| anyhow::anyhow!("press_key Enter failed: {}", e))?;
                return Ok(true);
            }
            Err(_) => continue,
        }
    }
    Ok(false)
}

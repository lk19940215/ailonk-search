use eoka::Page;

/// Try to click an element specified by a "trigger" string.
///
/// The trigger can be:
/// - A CSS selector (detected by leading `.`, `#`, `[`, or containing `::`)
/// - A button/link text (any other string)
///
/// Returns true if an element was found and clicked.
pub async fn click_trigger(page: &Page, trigger: &str) -> bool {
    let is_selector = trigger.starts_with('.')
        || trigger.starts_with('#')
        || trigger.starts_with('[')
        || trigger.contains("::");

    tracing::debug!(trigger = %trigger, is_selector, "Classifying trigger");

    if is_selector {
        if page.try_click(trigger).await.unwrap_or(false) {
            tracing::info!(selector = %trigger, "Clicked via CSS selector");
            return true;
        }
        tracing::debug!(selector = %trigger, "CSS selector miss, trying text match");
    }

    if page.try_click_by_text(trigger).await.unwrap_or(false) {
        tracing::info!(text = %trigger, "Clicked via exact text match");
        return true;
    }

    if click_by_text_contains(page, trigger).await {
        tracing::info!(text = %trigger, "Clicked via fuzzy text match");
        return true;
    }

    tracing::warn!(trigger = %trigger, "All click methods failed for trigger");
    false
}

/// Click the first element whose text content contains `needle` (case-insensitive).
pub async fn click_by_text_contains(page: &Page, needle: &str) -> bool {
    let js = format!(
        r#"(() => {{
            const needle = {}.toLowerCase();
            for (const el of document.querySelectorAll('a, button, [role="button"], input[type="submit"], span[onclick], div[onclick]')) {{
                const text = (el.textContent || el.value || '').trim().toLowerCase();
                if (text.includes(needle)) {{
                    el.click();
                    return true;
                }}
            }}
            return false;
        }})()"#,
        serde_json::json!(needle)
    );
    page.evaluate::<bool>(&js).await.unwrap_or(false)
}


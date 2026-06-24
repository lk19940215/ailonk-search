use eoka::Page;
use super::signals::{CAPTCHA_KEYWORDS, CAPTCHA_URL_SIGNALS};

pub fn detect_captcha(title: &str, url: &str) -> bool {
    let t = title.to_lowercase();
    let u = url.to_lowercase();
    CAPTCHA_KEYWORDS.iter().any(|kw| t.contains(kw))
        || CAPTCHA_URL_SIGNALS.iter().any(|sig| u.contains(sig))
}

/// Single-pass body text check — one `page.text()` call instead of per-keyword `text_exists`.
/// Also checks iframe URLs for CAPTCHA frames (covers cross-origin challenges with minimal parent text).
pub async fn is_captcha_present(page: &Page) -> bool {
    let title = page.title().await.unwrap_or_default();
    let url = page.url().await.unwrap_or_default();
    if detect_captcha(&title, &url) {
        return true;
    }

    let body_text = page.text().await.unwrap_or_default().to_lowercase();
    if CAPTCHA_KEYWORDS.iter().any(|kw| body_text.contains(kw)) {
        return true;
    }

    // Check if iframes contain known CAPTCHA URLs
    if let Ok(frames) = page.frames().await {
        for frame in &frames {
            let frame_url = frame.url.to_lowercase();
            if frame_url.contains("challenges.cloudflare")
                || frame_url.contains("turnstile")
                || frame_url.contains("recaptcha/api")
                || frame_url.contains("hcaptcha.com")
            {
                return true;
            }
        }
    }

    false
}

const CAPTCHA_IFRAME_SELECTORS: &[&str] = &[
    "iframe[src*=\"challenges.cloudflare\"]",
    "iframe[src*=\"turnstile\"]",
    "iframe[src*=\"recaptcha\"]",
    "iframe[title*=\"reCAPTCHA\"]",
    "iframe[src*=\"hcaptcha\"]",
    "iframe[src*=\"challenge\"]",
];

pub async fn attempt_resolve_captcha(page: &Page) -> bool {
    tracing::info!("Attempting CAPTCHA resolution...");

    // Strategy 1: Try in-frame checkbox click via evaluate_in_frame
    if try_iframe_interaction(page).await {
        return true;
    }

    // Strategy 2: Click verification buttons with human-like behavior
    // try_click_by_text is case-insensitive, no need for duplicate casing
    let verify_texts = ["Verify you are human", "Verify", "Continue", "确认", "验证", "继续"];
    for text in &verify_texts {
        if page.try_click_by_text(text).await.unwrap_or(false) {
            tracing::info!("Clicked verification button '{}'", text);
            if wait_for_captcha_resolution(page).await {
                tracing::info!("CAPTCHA resolved after clicking '{}'", text);
                return true;
            }
            // Stop after first successful click + wait — avoid redundant attempts
            break;
        }
    }

    // Strategy 3: Cloudflare non-iframe button selectors + form buttons
    let button_selectors = [
        "#challenge-stage button",
        ".ctp-checkbox-container",
        "#challenge-form button",
        "#challenge-form input[type='submit']",
        "button[id*='verify']",
    ];
    for sel in &button_selectors {
        if page.try_click(sel).await.unwrap_or(false) {
            tracing::info!("Clicked button: {}", sel);
            if wait_for_captcha_resolution(page).await {
                tracing::info!("CAPTCHA resolved after button click");
                return true;
            }
        }
    }

    // Strategy 4: Passive wait for auto-resolution (Turnstile, some reCAPTCHA v3)
    tracing::info!("Passive wait for potential auto-resolution...");
    page.wait(6000).await;
    if !is_captcha_present(page).await {
        tracing::info!("CAPTCHA auto-resolved");
        return true;
    }

    tracing::warn!("CAPTCHA resolution failed");
    false
}

/// Try to interact with CAPTCHA inside iframes using evaluate_in_frame.
/// Falls back to coordinate-based click if frame interaction fails.
async fn try_iframe_interaction(page: &Page) -> bool {
    let frames = page.frames().await.unwrap_or_default();

    for frame_info in &frames {
        let frame_url = frame_info.url.to_lowercase();
        let is_captcha_frame = frame_url.contains("challenges.cloudflare")
            || frame_url.contains("turnstile")
            || frame_url.contains("recaptcha")
            || frame_url.contains("hcaptcha")
            || frame_url.contains("challenge");

        if !is_captcha_frame {
            continue;
        }

        tracing::info!(frame_url = %frame_info.url, "Found CAPTCHA iframe, attempting in-frame click");

        // Try clicking checkbox inside frame via JS
        let click_js = r#"
            (function() {
                var cb = document.querySelector('input[type="checkbox"], .recaptcha-checkbox, .ctp-checkbox');
                if (cb) { cb.click(); return true; }
                var btn = document.querySelector('button, [role="button"]');
                if (btn) { btn.click(); return true; }
                return false;
            })()
        "#;

        if let Some(frame_selector) = build_frame_selector(&frame_info.url) {
            match page.evaluate_in_frame(&frame_selector, click_js).await {
                Ok(clicked) => {
                    let clicked_bool: bool = clicked;
                    if clicked_bool {
                        tracing::info!("Clicked inside CAPTCHA iframe via evaluate_in_frame");
                        if wait_for_captcha_resolution(page).await {
                            return true;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, "evaluate_in_frame failed, trying coordinate click");
                }
            }
        }
    }

    // Fallback: coordinate-based click on iframe center (original strategy)
    let iframe_rect_js = format!(
        r#"(() => {{
            const selectors = {:?};
            for (const sel of selectors) {{
                const frame = document.querySelector(sel);
                if (frame) {{
                    const r = frame.getBoundingClientRect();
                    if (r.width > 0 && r.height > 0) {{
                        return {{ x: r.x + r.width / 2, y: r.y + r.height / 2, found: sel }};
                    }}
                }}
            }}
            return null;
        }})()"#,
        CAPTCHA_IFRAME_SELECTORS
    );

    #[derive(serde::Deserialize)]
    struct IframeRect { x: f64, y: f64, found: String }

    if let Ok(Some(rect)) = super::input::extract::<Option<IframeRect>>(page, &iframe_rect_js).await {
        tracing::info!(selector = %rect.found, "CAPTCHA iframe found, clicking center coordinates");
        page.click_at(rect.x, rect.y).await.ok();
        if wait_for_captcha_resolution(page).await {
            tracing::info!("CAPTCHA resolved after iframe coordinate click");
            return true;
        }
    }

    false
}

/// Build a CSS selector to target the specific iframe by its known URL.
/// Prefers exact `src` match over generic pattern match for precision.
fn build_frame_selector(frame_url: &str) -> Option<String> {
    // Prefer targeting by exact src URL (avoids hitting wrong iframe among multiple matches)
    if !frame_url.is_empty() && frame_url != "about:blank" {
        return Some(format!("iframe[src=\"{}\"]", frame_url.replace('"', r#"\""#)));
    }
    // Fallback to pattern-based selector
    let url_lower = frame_url.to_lowercase();
    for sel in CAPTCHA_IFRAME_SELECTORS {
        if let Some(src_pattern) = sel.strip_prefix("iframe[src*=\"").and_then(|s| s.strip_suffix("\"]")) {
            if url_lower.contains(src_pattern) {
                return Some(sel.to_string());
            }
        }
    }
    None
}

/// Smart wait after a click action — uses URL change detection instead of fixed sleep.
async fn wait_for_captcha_resolution(page: &Page) -> bool {
    let initial_url = page.url().await.unwrap_or_default();

    // Short wait for immediate resolution
    page.wait(2000).await;
    if !is_captcha_present(page).await {
        return true;
    }

    // Check if URL changed (redirect after CAPTCHA)
    let current_url = page.url().await.unwrap_or_default();
    if current_url != initial_url {
        let _ = page.wait_for_network_idle(500, 5000).await;
        return !is_captcha_present(page).await;
    }

    // Longer wait for delayed resolution (Turnstile verification)
    page.wait(4000).await;
    !is_captcha_present(page).await
}

pub async fn resolve_captcha_loop(page: &Page, max_rounds: u32) -> anyhow::Result<bool> {
    if !is_captcha_present(page).await {
        return Ok(false);
    }
    tracing::warn!("CAPTCHA detected, starting resolution loop (max {} rounds)", max_rounds);

    for round in 1..=max_rounds {
        tracing::info!(round, "CAPTCHA resolution attempt");
        if attempt_resolve_captcha(page).await {
            return Ok(true);
        }
        if round < max_rounds {
            page.wait((round as u64) * 3000).await;
        }
    }

    anyhow::bail!(
        "CAPTCHA could not be resolved after {} attempts. \
         Try connecting your Chrome with login state for better results.",
        max_rounds
    )
}

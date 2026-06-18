use eoka::Page;

pub fn validate_url(url: &str) -> anyhow::Result<()> {
    let lower = url.to_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        anyhow::bail!("URL scheme not allowed (only http/https): {}", url);
    }
    Ok(())
}

pub fn validate_file_path(path: &str) -> anyhow::Result<()> {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        anyhow::bail!("Absolute paths not allowed: {}", path);
    }
    for component in p.components() {
        if matches!(component, std::path::Component::ParentDir) {
            anyhow::bail!("Path traversal (..) not allowed: {}", path);
        }
    }
    Ok(())
}

pub async fn navigate(page: &Page, url: &str, timeout_secs: u64) -> anyhow::Result<()> {
    page.goto(url).await
        .map_err(|e| anyhow::anyhow!("Navigation failed for {}: {}", url, e))?;

    page.wait_for("body", timeout_secs * 1000).await.ok();

    // Try common content container selectors for better readiness detection
    for sel in &["article", "main", "[role='main']", ".content", "#content"] {
        if page.wait_for(sel, 2000).await.is_ok() {
            break;
        }
    }

    let idle_timeout = (timeout_secs * 1000).min(6000);
    match page.wait_for_network_idle(500, idle_timeout).await {
        Ok(_) => {}
        Err(_) => {
            tracing::debug!(url, "Network not fully idle, proceeding with DOM-ready content");
        }
    }
    Ok(())
}

pub async fn extract<T: serde::de::DeserializeOwned>(
    page: &Page,
    js: &str,
) -> anyhow::Result<T> {
    page.evaluate(js).await
        .map_err(|e| anyhow::anyhow!("JS evaluation failed: {}", e))
}

/// Fill search box and submit. Uses eoka's human-like typing (Bezier + variable delays).
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
                page.human_fill(sel, text).await
                    .map_err(|e| anyhow::anyhow!("human_fill failed on {}: {}", sel, e))?;
                page.wait(300).await;
                page.press_key("Enter").await
                    .map_err(|e| anyhow::anyhow!("press_key Enter failed: {}", e))?;
                return Ok(true);
            }
            Err(_) => continue,
        }
    }
    Ok(false)
}

const CAPTCHA_KEYWORDS: &[&str] = &[
    "unusual traffic",
    "are not a robot",
    "captcha",
    "one last step",
    "verify you are human",
    "最后一步",
    "请解决",
    "人机验证",
    "异常流量",
];

const CAPTCHA_URL_SIGNALS: &[&str] = &["/sorry/", "captcha", "challenge"];

pub fn detect_captcha(title: &str, url: &str) -> bool {
    let t = title.to_lowercase();
    let u = url.to_lowercase();
    CAPTCHA_KEYWORDS.iter().any(|kw| t.contains(kw))
        || CAPTCHA_URL_SIGNALS.iter().any(|sig| u.contains(sig))
}

pub async fn is_captcha_present(page: &Page) -> bool {
    let title = page.title().await.unwrap_or_default();
    let url = page.url().await.unwrap_or_default();
    if detect_captcha(&title, &url) {
        return true;
    }
    for kw in CAPTCHA_KEYWORDS {
        if page.text_exists(kw).await {
            return true;
        }
    }
    false
}

pub async fn attempt_resolve_captcha(page: &Page) -> bool {
    tracing::info!("Attempting CAPTCHA resolution...");

    // Strategy 1: Click CAPTCHA iframe center
    let iframe_rect_js = r#"
        (() => {
            const selectors = [
                'iframe[src*="challenges.cloudflare"]',
                'iframe[src*="turnstile"]',
                'iframe[src*="recaptcha"]',
                'iframe[title*="reCAPTCHA"]',
                'iframe[src*="challenge"]',
            ];
            for (const sel of selectors) {
                const frame = document.querySelector(sel);
                if (frame) {
                    const r = frame.getBoundingClientRect();
                    if (r.width > 0 && r.height > 0) {
                        return { x: r.x + r.width / 2, y: r.y + r.height / 2, found: sel };
                    }
                }
            }
            return null;
        })()
    "#;

    #[derive(serde::Deserialize)]
    struct IframeRect { x: f64, y: f64, found: String }

    if let Ok(Some(rect)) = extract::<Option<IframeRect>>(page, iframe_rect_js).await {
        tracing::info!(selector = %rect.found, "CAPTCHA iframe found, clicking");
        page.click_at(rect.x, rect.y).await.ok();
        page.wait(6000).await;
        if !is_captcha_present(page).await {
            tracing::info!("CAPTCHA resolved after iframe click");
            return true;
        }
    }

    // Strategy 2: Click verification buttons by text
    let verify_texts = ["Verify", "Continue", "verify", "continue", "确认", "验证", "继续"];
    for text in &verify_texts {
        if page.try_click_by_text(text).await.unwrap_or(false) {
            page.wait(4000).await;
            if !is_captcha_present(page).await {
                tracing::info!("CAPTCHA resolved after clicking '{}'", text);
                return true;
            }
        }
    }

    // Strategy 3: Try form submit buttons
    let button_selectors = [
        "#challenge-form button",
        "#challenge-form input[type='submit']",
        "button[id*='verify']",
    ];
    for sel in &button_selectors {
        if page.try_click(sel).await.unwrap_or(false) {
            page.wait(4000).await;
            if !is_captcha_present(page).await {
                tracing::info!("CAPTCHA resolved after button click");
                return true;
            }
        }
    }

    // Strategy 4: Passive wait
    tracing::info!("Passive wait for potential auto-resolution...");
    page.wait(6000).await;
    if !is_captcha_present(page).await {
        tracing::info!("CAPTCHA auto-resolved");
        return true;
    }

    tracing::warn!("CAPTCHA resolution failed");
    false
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

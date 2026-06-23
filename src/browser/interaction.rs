use eoka::Page;

pub fn validate_url(url_str: &str, allow_private: bool) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url_str)
        .map_err(|e| anyhow::anyhow!("Invalid URL '{}': {}", url_str, e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        s => anyhow::bail!("URL scheme '{}' not allowed (only http/https)", s),
    }

    if allow_private {
        return Ok(());
    }

    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => {
            if ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
            {
                anyhow::bail!("Private/internal IPs are not allowed: {}. Use --allow-private-urls to override.", url_str);
            }
        }
        Some(url::Host::Ipv6(ip)) => {
            if ip.is_loopback()
                || ip.is_unspecified()
                || is_private_ipv6(&ip)
            {
                anyhow::bail!("Private/internal IPs are not allowed: {}. Use --allow-private-urls to override.", url_str);
            }
            if let Some(mapped) = ip.to_ipv4_mapped() {
                if mapped.is_loopback() || mapped.is_private() || mapped.is_link_local() {
                    anyhow::bail!("Private/internal IPs are not allowed: {}. Use --allow-private-urls to override.", url_str);
                }
            }
        }
        Some(url::Host::Domain(domain)) => {
            let d = domain.to_lowercase();
            if d == "localhost"
                || d.ends_with(".local")
                || d.ends_with(".internal")
            {
                anyhow::bail!("Private/internal domains are not allowed: {}. Use --allow-private-urls to override.", url_str);
            }
        }
        None => {
            anyhow::bail!("URL has no host: {}", url_str);
        }
    }
    Ok(())
}

fn is_private_ipv6(ip: &std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    // fc00::/7 (unique local)
    (segments[0] & 0xfe00) == 0xfc00
    // fe80::/10 (link-local)
    || (segments[0] & 0xffc0) == 0xfe80
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

/// Navigate to URL and wait for page to be ready for content extraction.
///
/// Strategy:
/// 1. goto (waits for load event internally)
/// 2. Wait for body element
/// 3. Wait for network idle (8s timeout — covers both SSR and SPA API calls)
/// 4. If content is still minimal (<200 chars), poll for SPA rendering completion
pub async fn navigate(page: &Page, url: &str, timeout_secs: u64) -> anyhow::Result<()> {
    page.goto(url).await
        .map_err(|e| anyhow::anyhow!("Navigation failed for {}: {}", url, e))?;

    page.wait_for("body", timeout_secs * 1000).await.ok();

    match page.wait_for_network_idle(500, 8000).await {
        Ok(_) => {}
        Err(_) => {
            tracing::debug!(url, "Network not fully idle after 8s, proceeding");
        }
    }

    let text_len: usize = page
        .evaluate("(document.body.innerText || '').length")
        .await
        .unwrap_or(0);

    if text_len < 200 {
        tracing::debug!(url, text_len, "Content minimal after network idle, waiting for SPA render");
        let mut last_len = text_len;
        for round in 1..=4 {
            page.wait(1500).await;
            let new_len: usize = page
                .evaluate("(document.body.innerText || '').length")
                .await
                .unwrap_or(0);
            if new_len > 200 {
                tracing::debug!(url, new_len, round, "SPA content appeared");
                let _ = page.wait_for_network_idle(500, 3000).await;
                break;
            }
            if new_len <= last_len {
                break;
            }
            last_len = new_len;
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
    "access denied",
    "security check",
    "最后一步",
    "请解决",
    "人机验证",
    "异常流量",
    "百度安全验证",
    "安全验证",
    "请完成验证",
    "请完成下方验证",
    "网络不给力",
    "访问异常",
];

const CAPTCHA_URL_SIGNALS: &[&str] = &[
    "/sorry/", "captcha", "challenge",
    "wappass.baidu.com", "/passport/", "/verify/",
];

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

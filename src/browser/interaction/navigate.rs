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
    (segments[0] & 0xfe00) == 0xfc00
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
/// 3. Wait for network idle (covers API calls for SPAs)
/// 4. If content is still minimal, use MutationObserver to wait for DOM stability
///    (200ms of no DOM changes = rendering complete)
pub async fn navigate(page: &Page, url: &str, timeout_secs: u64) -> anyhow::Result<()> {
    page.goto(url).await
        .map_err(|e| anyhow::anyhow!("Navigation failed for {}: {}", url, e))?;

    page.wait_for("body", timeout_secs * 1000).await.ok();

    let _ = page.wait_for_network_idle(500, 8000).await;

    let text_len: usize = page
        .evaluate_sync("(document.body.innerText || '').length")
        .await
        .unwrap_or(0);

    if text_len < 200 {
        tracing::debug!(url, text_len, "Content minimal, waiting for DOM mutation-idle");
        let _: bool = page.evaluate(MUTATION_IDLE_JS).await.unwrap_or(true);
        let _ = page.wait_for_network_idle(500, 3000).await;
    }

    Ok(())
}

/// JS Promise that resolves when the DOM stabilizes (200ms of no mutations),
/// with an 8s safety timeout. Uses MutationObserver for precise SPA render detection.
const MUTATION_IDLE_JS: &str = r#"new Promise(r=>{
let t=setTimeout(()=>r(!0),200);
const o=new MutationObserver(()=>{clearTimeout(t);t=setTimeout(()=>{o.disconnect();r(!0)},200)});
o.observe(document.body,{childList:!0,subtree:!0,characterData:!0});
setTimeout(()=>{o.disconnect();r(!0)},8000)
})"#;

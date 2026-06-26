use eoka::Page;
use std::time::Duration;

use super::auth::{self, AuthPageType, AuthResult};
use super::target_watcher::{TabSnapshot, TargetWatcher};

const URL_SETTLE_POLL_MS: u64 = 200;
const PAGE_BODY_WAIT_MS: u64 = 8000;
const PAGE_IDLE_WAIT_MS: u64 = 5000;
const PAGE_IDLE_CHECK_MS: u64 = 500;

/// Wait for a page's URL to become non-empty and non-`about:blank`.
/// Returns the settled URL (may still be blank if timeout is reached).
pub async fn wait_for_url_settle(page: &Page, timeout_secs: u64) -> String {
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let url = page.url().await.unwrap_or_default();
        if !url.is_empty() && url != "about:blank" {
            return url;
        }
        if tokio::time::Instant::now() >= deadline {
            return url;
        }
        tokio::time::sleep(std::time::Duration::from_millis(URL_SETTLE_POLL_MS)).await;
    }
}

/// Prepare an attached popup page: wait for DOM, network idle, and URL settle.
/// Returns the settled popup URL.
pub async fn prepare_popup_page(page: &Page) -> String {
    let _ = page.wait_for("body", PAGE_BODY_WAIT_MS).await;
    let _ = page.wait_for_network_idle(PAGE_IDLE_CHECK_MS, PAGE_IDLE_WAIT_MS).await;

    let url = page.url().await.unwrap_or_default();
    if url.is_empty() || url == "about:blank" {
        let settled = wait_for_url_settle(page, 5).await;
        if settled != url {
            let _ = page.wait_for_network_idle(PAGE_IDLE_CHECK_MS, 3000).await;
        }
        settled
    } else {
        url
    }
}

const REDIRECT_POLL_MS: u64 = 200;

/// Outcome of the post-click race between redirect detection and popup detection.
pub enum PostClickEvent {
    /// Main page URL changed (possibly auto-reauth completed the popup).
    Redirect(String),
    /// A new tab/popup was detected.
    Popup(TabSnapshot),
    /// Neither redirect nor popup within the deadline.
    Timeout,
}

/// Race redirect detection against popup detection after an SSO button click.
///
/// Uses `tokio::select!` to run both monitors concurrently, returning
/// whichever event fires first. This ensures popups are intercepted
/// immediately rather than waiting for a redirect timeout.
pub async fn race_redirect_vs_popup(
    page: &Page,
    pre_click_url: &str,
    watcher: &TargetWatcher,
    timeout_secs: u64,
) -> PostClickEvent {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    let redirect_fut = {
        let pre = pre_click_url.to_owned();
        async move {
            while tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(REDIRECT_POLL_MS)).await;
                let cur = page.url().await.unwrap_or_default();
                if cur != pre {
                    return PostClickEvent::Redirect(cur);
                }
            }
            PostClickEvent::Timeout
        }
    };

    let popup_fut = async {
        match watcher.wait_for_new(timeout_secs).await {
            Some(p) => PostClickEvent::Popup(p),
            None => PostClickEvent::Timeout,
        }
    };

    tokio::select! {
        r = redirect_fut => r,
        r = popup_fut => r,
    }
}

/// Result of interacting with an auth page (shared between authorize and popup_handler).
pub struct AuthInteraction {
    pub success: bool,
    pub message: String,
    pub accounts: Vec<String>,
}

/// Interact with an already-attached auth page: list accounts if applicable,
/// then call the full auth handler with preferred_account.
///
/// This is the shared interaction logic used by both `handle_auth_popup`
/// (full popup lifecycle) and `popup_handler` (pre-attached popup).
pub async fn interact_with_auth_page(
    page: &Page,
    auth_type: &AuthPageType,
    preferred_account: Option<&str>,
) -> AuthInteraction {
    let accounts = if auth_type.has_account_list() {
        auth::list_google_accounts(page)
            .await
            .into_iter()
            .map(|a| a.email)
            .collect()
    } else {
        vec![]
    };

    let result: AuthResult =
        auth::click_authorize_with_account(page, auth_type, preferred_account).await;

    if !result.success {
        tracing::warn!(
            auth_type = %auth_type, msg = %result.message,
            "Auth interaction failed"
        );
    }

    AuthInteraction {
        success: result.success,
        message: result.message,
        accounts,
    }
}

#[allow(dead_code)]
pub struct AuthPopupOutcome {
    pub success: bool,
    pub auth_type: AuthPageType,
    pub popup_url: String,
    pub message: String,
    pub closed: bool,
    pub accounts: Vec<String>,
}

/// Attach to a popup, detect its auth type, handle it, and wait for close.
///
/// Full popup lifecycle: attach → prepare → detect → interact → close.
/// For cases where the page is already attached, use `interact_with_auth_page` directly.
pub async fn handle_auth_popup(
    watcher: &TargetWatcher,
    popup: &TabSnapshot,
    preferred_account: Option<&str>,
    close_timeout_secs: u64,
) -> anyhow::Result<AuthPopupOutcome> {
    let popup_page = watcher.attach(&popup.id).await?;
    let popup_url = prepare_popup_page(&popup_page).await;

    let popup_auth = auth::detect_auth_page(&popup_page).await;
    tracing::info!(
        popup_url = %popup_url, auth_type = %popup_auth,
        "Popup auth detected"
    );

    let (success, message, accounts) = if matches!(popup_auth, AuthPageType::NotAuth) {
        (false, "Popup opened but is not an authorization page.".into(), vec![])
    } else {
        let interaction = interact_with_auth_page(&popup_page, &popup_auth, preferred_account).await;
        (interaction.success, interaction.message, interaction.accounts)
    };

    let closed = watcher.wait_for_close(&popup.id, close_timeout_secs).await;
    if !closed {
        tracing::warn!(popup_id = %popup.id, "Popup did not close within timeout");
    }

    Ok(AuthPopupOutcome {
        success: success && closed,
        auth_type: popup_auth,
        popup_url,
        message,
        closed,
        accounts,
    })
}

use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData};

use crate::browser::interaction;
use crate::browser::interaction::popup_flow::{self, PostClickEvent};
use crate::browser::interaction::target_watcher::TargetWatcher;
use crate::browser::manager::BrowserManager;
use crate::search::engine::to_mcp_error;

use super::is_fatal_cdp_error_anyhow;
use super::tools;

/// Run the click_authorize flow: navigate, detect auth pages, handle popups and SSO loops.
/// Note: outer `with_hard_timeout` in mod.rs provides spawn-isolated timeout + unhealthy marking.
pub async fn handle_click_authorize(
    bm: Arc<BrowserManager>,
    params: tools::ClickAuthorizeParams,
    allow_private_urls: bool,
) -> Result<CallToolResult, ErrorData> {
    interaction::validate_url(&params.url, allow_private_urls).map_err(to_mcp_error)?;
    click_authorize_inner(bm, params, allow_private_urls).await
}

async fn click_authorize_inner(
    bm: Arc<BrowserManager>,
    params: tools::ClickAuthorizeParams,
    _allow_private_urls: bool,
) -> Result<CallToolResult, ErrorData> {
    let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;
    let watcher = TargetWatcher::new(bm.clone())
        .await
        .map_err(to_mcp_error)?;

    let timeout = params.timeout;
    let preferred_account = params.preferred_account.as_deref();

    let result = async {
        interaction::navigate(tab.page(), &params.url, 15).await?;
        interaction::handle_consent(tab.page(), "").await?;

        let initial_auth = interaction::auth::detect_auth_page_with_target(
            tab.page(),
            Some(&params.url),
        )
        .await;
        let current_url = tab.page().url().await.unwrap_or_default();
        tracing::info!(url = %current_url, auth_type = %initial_auth, "Auth page detection");

        match initial_auth {
            interaction::auth::AuthPageType::NotAuth => {
                handle_not_auth_with_popup(&watcher, tab.page(), timeout, preferred_account).await
            }
            ref t if t.needs_sso_loop() => {
                sso_loop(
                    tab.page(),
                    &bm,
                    &params.url,
                    &initial_auth,
                    timeout,
                    preferred_account,
                )
                .await
            }
            _ => {
                Ok(interaction::auth::click_authorize_with_account(
                    tab.page(),
                    &initial_auth,
                    preferred_account,
                )
                .await)
            }
        }
    }
    .await;

    tab.close().await;
    format_auth_result(result, &bm)
}

/// NotAuth initial page: wait for a popup, handle it if it appears.
async fn handle_not_auth_with_popup(
    watcher: &TargetWatcher,
    page: &eoka::Page,
    timeout: u64,
    preferred_account: Option<&str>,
) -> anyhow::Result<interaction::auth::AuthResult> {
    if let Some(popup) = watcher.wait_for_new(timeout).await {
        tracing::info!(popup_id = %popup.id, url = %popup.url, "Popup detected");
        let outcome = popup_flow::handle_auth_popup(watcher, &popup, preferred_account, timeout)
            .await?;
        Ok(interaction::auth::AuthResult {
            success: outcome.success,
            auth_type: outcome.auth_type,
            final_url: page.url().await.unwrap_or_default(),
            message: outcome.message,
        })
    } else {
        Ok(interaction::auth::AuthResult {
            success: true,
            auth_type: interaction::auth::AuthPageType::NotAuth,
            final_url: page.url().await.unwrap_or_default(),
            message: "Page does not require authorization.".to_string(),
        })
    }
}

/// SSO click-and-wait loop for CustomSso / GenericLogin pages.
///
/// Each iteration: detect page type → dispatch handler or click SSO button →
/// race redirect vs popup → handle result → repeat until target reached or timeout.
async fn sso_loop(
    page: &eoka::Page,
    bm: &Arc<BrowserManager>,
    target_url: &str,
    initial_auth: &interaction::auth::AuthPageType,
    timeout: u64,
    preferred_account: Option<&str>,
) -> anyhow::Result<interaction::auth::AuthResult> {
    let mut step = 0u8;
    let mut last_event = String::from("none");
    let mut click_failures = 0u8;
    let global_deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);

    loop {
        if tokio::time::Instant::now() > global_deadline {
            tracing::warn!(timeout, last_event = %last_event, step, "SSO loop timed out");
            let final_url = page.url().await.unwrap_or_default();
            return Ok(interaction::auth::AuthResult {
                success: false,
                auth_type: initial_auth.clone(),
                final_url,
                message: format!("Auth timed out ({}s). [{}]", timeout, last_event),
            });
        }

        page.wait_for("body", 5000).await.ok();
        let _ = page.wait_for_network_idle(500, 8000).await;

        let page_url = page.url().await.unwrap_or_default();
        let page_auth =
            interaction::auth::detect_auth_page_with_target(page, Some(target_url)).await;
        step += 1;
        tracing::debug!(step, url = %page_url, auth = %page_auth, "SSO step");

        match page_auth {
            interaction::auth::AuthPageType::NotAuth => {
                if let Some(result) =
                    check_target_reached(page, target_url, initial_auth, step, &last_event).await
                {
                    return Ok(result);
                }
                let final_url = page.url().await.unwrap_or_default();
                return Ok(interaction::auth::AuthResult {
                    success: false,
                    auth_type: initial_auth.clone(),
                    final_url,
                    message: format!("Page is not auth but target not reachable. [{}]", last_event),
                });
            }
            ref auth if auth.needs_interactive_handler() => {
                tracing::debug!(auth = %auth, step, "Interactive auth on main page");
                let result = interaction::auth::click_authorize_with_account(
                    page, auth, preferred_account,
                )
                .await;
                if result.success {
                    last_event = format!("interactive({})", auth);
                }
                continue;
            }
            _ => {
                let pre_click_url = page.url().await.unwrap_or_default();
                let click_watcher = TargetWatcher::new(bm.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))?;

                let clicked = interaction::auth::try_sso_click(page).await;
                tracing::debug!(clicked, step, "SSO button click");

                if !clicked {
                    click_failures += 1;
                    if click_failures >= 3 {
                        let final_url = page.url().await.unwrap_or_default();
                        return Ok(interaction::auth::AuthResult {
                            success: false,
                            auth_type: initial_auth.clone(),
                            final_url,
                            message: "No clickable SSO button found after 3 attempts.".into(),
                        });
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                click_failures = 0;

                let remaining = global_deadline
                    .duration_since(tokio::time::Instant::now())
                    .as_secs()
                    .max(3);
                let race_secs = remaining.min(15);

                let event = popup_flow::race_redirect_vs_popup(
                    page,
                    &pre_click_url,
                    &click_watcher,
                    race_secs,
                )
                .await;

                match event {
                    PostClickEvent::Redirect(new_url) => {
                        tracing::info!(new_url = %new_url, "Redirected after click");
                        let url_preview: String = new_url.chars().take(80).collect();
                        last_event = format!("redirect({})", url_preview);
                        page.wait_for("body", 5000).await.ok();
                        let _ = page.wait_for_network_idle(500, 8000).await;
                    }
                    PostClickEvent::Popup(popup) => {
                        tracing::info!(
                            popup_id = %popup.id, url = %popup.url,
                            "Popup detected — actively handling"
                        );
                        match popup_flow::handle_auth_popup(
                            &click_watcher,
                            &popup,
                            preferred_account,
                            timeout,
                        )
                        .await
                        {
                            Ok(outcome) => {
                                last_event = format!(
                                    "popup_active(url={})",
                                    outcome.popup_url.chars().take(60).collect::<String>()
                                );
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "Popup attach/handle failed");
                                last_event = format!("popup_error({})", e);
                            }
                        }
                    }
                    PostClickEvent::Timeout => {
                        last_event = String::from("no_response");
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
    }
}

/// Check if we've reached the target URL, possibly navigating to it first.
async fn check_target_reached(
    page: &eoka::Page,
    target_url: &str,
    initial_auth: &interaction::auth::AuthPageType,
    step: u8,
    last_event: &str,
) -> Option<interaction::auth::AuthResult> {
    let page_url = page.url().await.unwrap_or_default();
    let at_target = page_url
        .to_lowercase()
        .trim_end_matches('/')
        == target_url.to_lowercase().trim_end_matches('/');

    if at_target {
        return Some(interaction::auth::AuthResult {
            success: true,
            auth_type: initial_auth.clone(),
            final_url: page_url,
            message: format!("Auth completed after {} steps. [{}]", step - 1, last_event),
        });
    }

    if interaction::navigate(page, target_url, 10).await.is_ok() {
        let nav_url = page.url().await.unwrap_or_default();
        let nav_auth =
            interaction::auth::detect_auth_page_with_target(page, Some(target_url)).await;
        if matches!(nav_auth, interaction::auth::AuthPageType::NotAuth) {
            return Some(interaction::auth::AuthResult {
                success: true,
                auth_type: initial_auth.clone(),
                final_url: nav_url,
                message: format!(
                    "Auth completed. Navigated to target after {} steps. [{}]",
                    step - 1,
                    last_event
                ),
            });
        }
    }

    None
}

fn format_auth_result(
    result: anyhow::Result<interaction::auth::AuthResult>,
    bm: &BrowserManager,
) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(auth_result) => {
            let status = match (&auth_result.auth_type, auth_result.success) {
                (interaction::auth::AuthPageType::NotAuth, true) => "no_auth_needed",
                (_, true) => "authorized",
                (_, false) => "manual_required",
            };
            let msg = format!(
                "Status: {}\nAuth type: {}\nFinal URL: {}\n\n{}\n\n{}",
                status,
                auth_result.auth_type,
                auth_result.final_url,
                auth_result.message,
                if auth_result.success {
                    "You can now use read_page to read the content at the target URL."
                } else {
                    "Authorization may require manual user intervention. Try sync_login if this is a login issue rather than an OAuth consent."
                }
            );
            Ok(CallToolResult::success(vec![Content::text(msg)]))
        }
        Err(e) => {
            tracing::error!(error = %e, "Authorization flow failed");
            if is_fatal_cdp_error_anyhow(&e) {
                bm.mark_unhealthy();
            }
            Err(to_mcp_error(format!("Authorization flow failed: {}", e)))
        }
    }
}

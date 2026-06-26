use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData};

use crate::browser::interaction;
use crate::browser::interaction::popup_flow;
use crate::browser::interaction::target_watcher::TargetWatcher;
use crate::browser::manager::BrowserManager;
use crate::search::engine::to_mcp_error;

use super::is_fatal_cdp_error_anyhow;
use super::tools;

/// General-purpose popup/new-tab handler.
///
/// Lifecycle:
/// 1. Navigate to `url`, take a baseline tab snapshot
/// 2. If `trigger` is provided, click that element on the page
/// 3. Monitor for new tabs (filtered by `popup_url_contains` if set)
/// 4. Attach to the popup, auto-detect its type, interact accordingly
/// 5. Wait for popup to close, return results + original page state
///
/// Note: outer `with_hard_timeout` in mod.rs provides spawn-isolated timeout + unhealthy marking.
pub async fn handle_popup(
    bm: Arc<BrowserManager>,
    params: tools::HandlePopupParams,
    allow_private_urls: bool,
) -> Result<CallToolResult, ErrorData> {
    interaction::validate_url(&params.url, allow_private_urls).map_err(to_mcp_error)?;
    handle_popup_inner(bm, params, allow_private_urls).await
}

async fn handle_popup_inner(
    bm: Arc<BrowserManager>,
    params: tools::HandlePopupParams,
    _allow_private_urls: bool,
) -> Result<CallToolResult, ErrorData> {
    let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;
    let watcher = TargetWatcher::with_filter(bm.clone(), params.popup_url_contains.clone())
        .await
        .map_err(to_mcp_error)?;
    let timeout = params.timeout;

    let result = async {
        tracing::info!(
            url = %params.url, trigger = ?params.trigger,
            popup_url_contains = ?params.popup_url_contains,
            has_preferred_account = params.preferred_account.is_some(),
            popup_click = ?params.popup_click,
            timeout, "handle_popup started"
        );

        interaction::navigate(tab.page(), &params.url, 15).await?;
        interaction::handle_consent(tab.page(), "").await?;
        let _ = tab.page().wait_for_network_idle(500, 5000).await;

        // Phase 1: Click trigger if specified
        if let Some(ref trigger) = params.trigger {
            let clicked = interaction::click::click_trigger(tab.page(), trigger).await;
            if !clicked {
                return Ok(PopupResult {
                    success: false,
                    popup_url: None,
                    popup_type: "none".to_string(),
                    final_page_url: tab.page().url().await.unwrap_or_default(),
                    message: format!("Could not find or click trigger element: {}", trigger),
                    accounts_found: vec![],
                });
            }
            tracing::info!(trigger = %trigger, "Trigger clicked, waiting for popup");
        }

        // Phase 2: Wait for popup
        let popup = match watcher.wait_for_new(timeout).await {
            Some(snap) => snap,
            None => {
                let current_url = tab.page().url().await.unwrap_or_default();
                let redirected = current_url != params.url;
                return Ok(PopupResult {
                    success: false,
                    popup_url: None,
                    popup_type: if redirected { "redirect" } else { "none" }.to_string(),
                    final_page_url: current_url,
                    message: if redirected {
                        "No popup detected, but page redirected.".to_string()
                    } else {
                        format!("No popup detected within {}s.", timeout)
                    },
                    accounts_found: vec![],
                });
            }
        };

        tracing::debug!(popup_id = %popup.id, popup_url = %popup.url, "Popup detected");

        let popup_page = watcher.attach(&popup.id).await?;
        let popup_url = popup_flow::prepare_popup_page(&popup_page).await;
        let popup_auth_type = interaction::auth::detect_auth_page(&popup_page).await;

        tracing::info!(popup_url = %popup_url, auth_type = %popup_auth_type, "Popup attached and analyzed");

        let preferred = params.preferred_account.as_deref();
        let popup_type_str = popup_auth_type.to_string();

        let (interaction_success, interaction_msg, accounts) = match popup_auth_type {
            interaction::auth::AuthPageType::NotAuth => {
                handle_non_auth_popup(&popup_page, params.popup_click.as_deref()).await
            }
            _ => {
                let r = popup_flow::interact_with_auth_page(
                    &popup_page, &popup_auth_type, preferred,
                ).await;
                (r.success, r.message, r.accounts)
            }
        };

        // Phase 4: Wait for close + settle
        let closed = watcher.wait_for_close(&popup.id, timeout).await;
        if !closed {
            tracing::warn!(popup_id = %popup.id, "Popup did not close within timeout");
        }
        if closed {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let _ = tab.page().wait_for_network_idle(500, 5000).await;
        }

        let final_url = tab.page().url().await.unwrap_or_default();
        let is_auth = !matches!(popup_auth_type, interaction::auth::AuthPageType::NotAuth);
        let effective_success = if is_auth {
            interaction_success && closed
        } else {
            interaction_success
        };

        tracing::debug!(
            interaction_success, closed, is_auth, effective_success,
            final_url = %final_url, popup_type = %popup_type_str,
            "handle_popup completed"
        );

        Ok(PopupResult {
            success: effective_success,
            popup_url: Some(popup_url),
            popup_type: popup_type_str,
            final_page_url: final_url,
            message: interaction_msg,
            accounts_found: accounts,
        })
    }
    .await;

    tab.close().await;

    match result {
        Ok(pr) => Ok(format_popup_result(pr)),
        Err(e) => {
            tracing::error!(error = %e, "Popup handling failed");
            if is_fatal_cdp_error_anyhow(&e) {
                bm.mark_unhealthy();
            }
            Err(to_mcp_error(format!("Popup handling failed: {}", e)))
        }
    }
}

/// Handle a non-auth popup: click specified element or return content preview.
async fn handle_non_auth_popup(
    popup_page: &eoka::Page,
    popup_click: Option<&str>,
) -> (bool, String, Vec<String>) {
    if let Some(click_target) = popup_click {
        let clicked = interaction::click::click_trigger(popup_page, click_target).await;
        if clicked {
            popup_page.wait(1500).await;
            let _ = popup_page.wait_for_network_idle(500, 3000).await;
            (true, format!("Clicked '{}' in popup.", click_target), vec![])
        } else {
            let title = popup_page
                .evaluate::<String>("document.title")
                .await
                .unwrap_or_default();
            (
                false,
                format!("Could not find '{}' in popup. Title: \"{}\"", click_target, title),
                vec![],
            )
        }
    } else {
        let title = popup_page
            .evaluate::<String>("document.title")
            .await
            .unwrap_or_default();
        let text = popup_page.text().await.unwrap_or_default();
        let preview: String = text.chars().take(500).collect();
        (
            true,
            format!("Popup detected (non-auth). Title: \"{}\". Content preview:\n{}", title, preview),
            vec![],
        )
    }
}

struct PopupResult {
    success: bool,
    popup_url: Option<String>,
    popup_type: String,
    final_page_url: String,
    message: String,
    accounts_found: Vec<String>,
}

fn format_popup_result(r: PopupResult) -> CallToolResult {
    let status = if r.success { "completed" } else { "incomplete" };
    let mut msg = format!(
        "Status: {}\nPopup type: {}\nFinal page URL: {}\n\n{}",
        status, r.popup_type, r.final_page_url, r.message
    );

    if let Some(ref popup_url) = r.popup_url {
        msg.push_str(&format!("\nPopup URL: {}", popup_url));
    }
    if !r.accounts_found.is_empty() {
        msg.push_str(&format!("\nAccounts found: {}", r.accounts_found.join(", ")));
    }
    if r.success {
        msg.push_str("\n\nPopup handled successfully. You can now use read_page on the target URL.");
    } else {
        msg.push_str("\n\nPopup interaction may require manual intervention or additional steps.");
    }

    CallToolResult::success(vec![Content::text(msg)])
}

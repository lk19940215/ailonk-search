use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ErrorData};

use crate::browser::interaction;
use crate::browser::manager::BrowserManager;
use crate::search::engine::to_mcp_error;

use super::is_fatal_cdp_error_anyhow;
use super::tools;

/// Run the click_authorize flow: navigate, detect auth pages, handle popups and SSO loops.
pub async fn handle_click_authorize(
    bm: Arc<BrowserManager>,
    params: tools::ClickAuthorizeParams,
    allow_private_urls: bool,
) -> Result<CallToolResult, ErrorData> {
    interaction::validate_url(&params.url, allow_private_urls).map_err(to_mcp_error)?;

    let popup_watcher = interaction::popup::PopupWatcher::new(bm.clone())
        .await
        .map_err(to_mcp_error)?;

    let tab = bm.tab_pool().acquire().await.map_err(to_mcp_error)?;

    let timeout = params.timeout;
    let result = async {
        interaction::navigate(tab.page(), &params.url, 15).await?;
        interaction::handle_consent(tab.page(), "").await?;

        let auth_type = interaction::auth::detect_auth_page_with_target(tab.page(), Some(&params.url)).await;
        let current_url = tab.page().url().await.unwrap_or_default();
        tracing::info!(url = %current_url, auth_type = %auth_type, "Auth page detection result");

        match auth_type {
            interaction::auth::AuthPageType::NotAuth => {
                // No auth detected on main page — check for popup
                if let Some(popup_id) = popup_watcher.wait_for_popup(timeout.min(8)).await {
                    tracing::info!(popup_id = %popup_id, "Popup detected, attaching");
                    let popup_page = popup_watcher.attach(&popup_id).await?;
                    let _ = popup_page.wait_for("body", 10000).await;
                    let _ = popup_page.wait_for_network_idle(500, 5000).await;
                    let popup_auth_type = interaction::auth::detect_auth_page(&popup_page).await;
                    let auth_result = if matches!(popup_auth_type, interaction::auth::AuthPageType::NotAuth) {
                        interaction::auth::AuthResult {
                            success: false,
                            auth_type: popup_auth_type,
                            final_url: popup_page.url().await.unwrap_or_default(),
                            message: "Popup opened but it's not an authorization page.".to_string(),
                        }
                    } else {
                        interaction::auth::click_authorize(&popup_page, &popup_auth_type).await
                    };
                    popup_watcher.wait_for_close(&popup_id, params.timeout).await;
                    Ok(auth_result)
                } else {
                    Ok(interaction::auth::AuthResult {
                        success: true,
                        auth_type: interaction::auth::AuthPageType::NotAuth,
                        final_url: tab.page().url().await.unwrap_or_default(),
                        message: "Page does not require authorization.".to_string(),
                    })
                }
            }
            // Unified handler for CustomSso and GenericLogin:
            // Simple loop: detect → click → wait for redirect → repeat until target reached
            interaction::auth::AuthPageType::CustomSso | interaction::auth::AuthPageType::GenericLogin => {
                let page = tab.page();

                const MAX_LOOPS: u8 = 4;
                let mut loop_count = 0u8;
                let mut last_race = String::from("none");

                loop {
                    loop_count += 1;
                    if loop_count > MAX_LOOPS {
                        let final_url = page.url().await.unwrap_or_default();
                        return Ok(interaction::auth::AuthResult {
                            success: false,
                            auth_type: auth_type.clone(),
                            final_url,
                            message: format!("Auth loop exceeded max iterations ({}). last_race={}", MAX_LOOPS, last_race),
                        });
                    }

                    let page_url = page.url().await.unwrap_or_default();
                    let page_auth = interaction::auth::detect_auth_page_with_target(page, Some(&params.url)).await;
                    tracing::info!(loop_count, url = %page_url, auth = %page_auth, "Auth loop iteration");

                    match page_auth {
                        interaction::auth::AuthPageType::NotAuth => {
                            // Check if we're at the target
                            let url_lower = page_url.to_lowercase();
                            let at_target = url_lower.trim_end_matches('/') == params.url.to_lowercase().trim_end_matches('/');
                            let not_login = !url_lower.contains("auth.html")
                                && !url_lower.contains("stlogin")
                                && !url_lower.contains("/login");
                            if at_target || (not_login && page_url != current_url) {
                                return Ok(interaction::auth::AuthResult {
                                    success: true,
                                    auth_type: auth_type.clone(),
                                    final_url: page_url,
                                    message: format!("Auth completed after {} steps.", loop_count - 1),
                                });
                            }
                            // Not at target but NotAuth — try navigating directly
                            if interaction::navigate(page, &params.url, 10).await.is_ok() {
                                let nav_url = page.url().await.unwrap_or_default();
                                if nav_url.to_lowercase().trim_end_matches('/') == params.url.to_lowercase().trim_end_matches('/') {
                                    return Ok(interaction::auth::AuthResult {
                                        success: true,
                                        auth_type: auth_type.clone(),
                                        final_url: nav_url,
                                        message: format!("Auth completed. Navigated to target after {} steps.", loop_count - 1),
                                    });
                                }
                            }
                            // Still can't reach target
                            let final_url = page.url().await.unwrap_or_default();
                            return Ok(interaction::auth::AuthResult {
                                success: false,
                                auth_type: auth_type.clone(),
                                final_url,
                                message: format!("Auth loop: page is not auth but target not reachable. last_race={}", last_race),
                            });
                        }
                        _ => {
                            // Auth page detected — click button, then wait for redirect
                            page.wait(2000).await;

                            let pre_click_url = page.url().await.unwrap_or_default();

                            let click_pw = interaction::popup::PopupWatcher::new(bm.clone())
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?;

                            let clicked = interaction::auth::try_sso_click(page).await;
                            tracing::info!(clicked, loop_count, "Auth button click");

                            if !clicked {
                                tracing::debug!(loop_count, "Click failed, will retry on next iteration");
                                tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                                continue;
                            }

                            // Wait for redirect (Chrome handles FedCM natively, auto-selects account)
                            // Poll URL frequently for fast detection instead of fixed waits
                            let mut redirected = false;
                            let redirect_deadline = tokio::time::Instant::now()
                                + std::time::Duration::from_secs(12);
                            while tokio::time::Instant::now() < redirect_deadline {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                let current = page.url().await.unwrap_or_default();
                                if current != pre_click_url {
                                    tracing::info!(new_url = %current, "Page redirected after click");
                                    redirected = true;
                                    last_race = String::from("redirect");
                                    break;
                                }
                            }

                            if !redirected {
                                // Check for popup (some flows open a new window)
                                if let Some(popup_id) = click_pw.wait_for_popup(3).await {
                                    if let Ok(popup_page) = click_pw.attach(&popup_id).await {
                                        let popup_url = popup_page.url().await.unwrap_or_default();
                                        let popup_auth = interaction::auth::detect_auth_page(&popup_page).await;
                                        tracing::info!(popup_url = %popup_url, popup_auth = %popup_auth, "Popup detected");

                                        if !matches!(popup_auth, interaction::auth::AuthPageType::NotAuth) {
                                            let _ = interaction::auth::click_authorize(&popup_page, &popup_auth).await;
                                        }
                                        click_pw.wait_for_close(&popup_id, timeout.min(15)).await;
                                        last_race = format!("popup(url={})", popup_url.chars().take(60).collect::<String>());
                                    }
                                } else {
                                    last_race = String::from("no_redirect_no_popup");
                                }

                                // After popup handling, wait for main page to settle
                                tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                                let _ = page.wait_for_network_idle(500, 3000).await;
                            }
                            // Loop back to re-detect auth state
                        }
                    }
                }
            }
            _ => {
                Ok(interaction::auth::click_authorize(tab.page(), &auth_type).await)
            }
        }
    }.await;

    tab.close().await;

    match result {
        Ok(auth_result) => {
            let status = match (&auth_result.auth_type, auth_result.success) {
                (interaction::auth::AuthPageType::NotAuth, _) => "no_auth_needed",
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
            if is_fatal_cdp_error_anyhow(&e) {
                bm.mark_unhealthy();
            }
            Err(to_mcp_error(format!("Authorization flow failed: {}", e)))
        }
    }
}

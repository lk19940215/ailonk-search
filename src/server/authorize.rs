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
            interaction::auth::AuthPageType::CustomSso | interaction::auth::AuthPageType::GenericLogin => {
                let page = tab.page();

                let mut step = 0u8;
                let mut last_event = String::from("none");
                let global_deadline = tokio::time::Instant::now()
                    + std::time::Duration::from_secs(timeout);

                loop {
                    if tokio::time::Instant::now() > global_deadline {
                        let final_url = page.url().await.unwrap_or_default();
                        return Ok(interaction::auth::AuthResult {
                            success: false,
                            auth_type: auth_type.clone(),
                            final_url,
                            message: format!("Auth timed out ({}s). last_event={}", timeout, last_event),
                        });
                    }

                    // Ensure page is fully loaded before detection
                    page.wait_for("body", 5000).await.ok();
                    let _ = page.wait_for_network_idle(500, 8000).await;

                    let page_url = page.url().await.unwrap_or_default();
                    let page_auth = interaction::auth::detect_auth_page_with_target(page, Some(&params.url)).await;
                    step += 1;
                    tracing::info!(step, url = %page_url, auth = %page_auth, "Auth step");

                    match page_auth {
                        interaction::auth::AuthPageType::NotAuth => {
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
                                    message: format!("Auth completed after {} steps.", step - 1),
                                });
                            }
                            if interaction::navigate(page, &params.url, 10).await.is_ok() {
                                let nav_url = page.url().await.unwrap_or_default();
                                if nav_url.to_lowercase().trim_end_matches('/') == params.url.to_lowercase().trim_end_matches('/') {
                                    return Ok(interaction::auth::AuthResult {
                                        success: true,
                                        auth_type: auth_type.clone(),
                                        final_url: nav_url,
                                        message: format!("Auth completed. Navigated to target after {} steps.", step - 1),
                                    });
                                }
                            }
                            let final_url = page.url().await.unwrap_or_default();
                            return Ok(interaction::auth::AuthResult {
                                success: false,
                                auth_type: auth_type.clone(),
                                final_url,
                                message: format!("Page is not auth but target not reachable. last_event={}", last_event),
                            });
                        }
                        _ => {
                            let pre_click_url = page.url().await.unwrap_or_default();

                            let click_pw = interaction::popup::PopupWatcher::new(bm.clone())
                                .await
                                .map_err(|e| anyhow::anyhow!("{}", e))?;

                            let clicked = interaction::auth::try_sso_click(page).await;
                            tracing::info!(clicked, step, "Auth button click");

                            if !clicked {
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                continue;
                            }

                            // Wait for page response: redirect or popup
                            let mut redirected = false;
                            let redirect_deadline = tokio::time::Instant::now()
                                + std::time::Duration::from_secs(15);
                            while tokio::time::Instant::now() < redirect_deadline {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                let current = page.url().await.unwrap_or_default();
                                if current != pre_click_url {
                                    tracing::info!(new_url = %current, "Redirected after click");
                                    redirected = true;
                                    last_event = String::from("redirect");
                                    break;
                                }
                            }

                            if redirected {
                                // Let new page fully load (including iframes)
                                page.wait_for("body", 5000).await.ok();
                                let _ = page.wait_for_network_idle(500, 8000).await;
                            } else if let Some(popup_id) = click_pw.wait_for_popup(3).await {
                                if let Ok(popup_page) = click_pw.attach(&popup_id).await {
                                    let popup_url = popup_page.url().await.unwrap_or_default();
                                    let popup_auth = interaction::auth::detect_auth_page(&popup_page).await;
                                    tracing::info!(popup_url = %popup_url, popup_auth = %popup_auth, "Popup detected");

                                    if !matches!(popup_auth, interaction::auth::AuthPageType::NotAuth) {
                                        let _ = interaction::auth::click_authorize(&popup_page, &popup_auth).await;
                                    }
                                    click_pw.wait_for_close(&popup_id, timeout.min(15)).await;
                                    last_event = format!("popup(url={})", popup_url.chars().take(60).collect::<String>());
                                }
                            } else {
                                last_event = String::from("no_response");
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            }
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

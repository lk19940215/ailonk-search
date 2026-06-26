use eoka::Page;
use super::signals::{AUTH_URL_PATTERNS, AUTH_CONSENT_BUTTON_TEXTS, SSO_URL_PATTERNS, SSO_REDIRECT_PARAMS, SSO_CONTENT_SIGNALS, AUTH_BUTTON_KEYWORDS};

#[derive(Debug, Clone)]
pub enum AuthPageType {
    GoogleOAuthConsent,
    GoogleAccountSelection,
    GoogleSamlRedirect,
    GenericOAuth,
    CustomSso,
    /// Any page with a detectable login/SSO button that doesn't match specific patterns
    GenericLogin,
    NotAuth,
}

impl AuthPageType {
    /// Whether this auth type should be handled by the full `click_authorize_with_account`
    /// pipeline (with preferred_account, account listing, etc.) rather than the
    /// generic `try_sso_click` loop.
    ///
    /// Server-layer code should use this instead of pattern-matching on specific variants.
    pub fn needs_interactive_handler(&self) -> bool {
        matches!(
            self,
            Self::GoogleOAuthConsent
                | Self::GoogleAccountSelection
                | Self::GoogleSamlRedirect
                | Self::GenericOAuth
        )
    }

    /// Whether this auth type requires an SSO click-and-wait loop
    /// (as opposed to a single-pass handler).
    pub fn needs_sso_loop(&self) -> bool {
        matches!(self, Self::CustomSso | Self::GenericLogin)
    }

    /// Whether this auth page type exposes a selectable account list.
    pub fn has_account_list(&self) -> bool {
        matches!(self, Self::GoogleAccountSelection)
    }
}

impl std::fmt::Display for AuthPageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GoogleOAuthConsent => write!(f, "google_oauth_consent"),
            Self::GoogleAccountSelection => write!(f, "google_account_selection"),
            Self::GoogleSamlRedirect => write!(f, "google_saml_redirect"),
            Self::GenericOAuth => write!(f, "generic_oauth"),
            Self::CustomSso => write!(f, "custom_sso"),
            Self::GenericLogin => write!(f, "generic_login"),
            Self::NotAuth => write!(f, "not_auth"),
        }
    }
}

#[derive(Debug)]
pub struct AuthResult {
    pub success: bool,
    pub auth_type: AuthPageType,
    pub final_url: String,
    pub message: String,
}

/// Detect if the current page is an authorization/login page.
/// Checks URL patterns first (fast path), then falls back to page content analysis.
/// `target_url` is the originally requested URL; if current URL differs from it,
/// generic login button detection is enabled (avoids false positives on content pages).
pub async fn detect_auth_page(page: &Page) -> AuthPageType {
    detect_auth_page_with_target(page, None).await
}

pub async fn detect_auth_page_with_target(page: &Page, target_url: Option<&str>) -> AuthPageType {
    let url = page.url().await.unwrap_or_default().to_lowercase();

    if url.contains("accounts.google.com/o/oauth2")
        || url.contains("accounts.google.com/signin/oauth")
        || url.contains("accounts.google.com/v3/signin/oauth")
    {
        return AuthPageType::GoogleOAuthConsent;
    }
    if url.contains("accounts.google.com/accountchooser")
        || url.contains("accounts.google.com/signin/selectaccount")
        || url.contains("accounts.google.com/v3/signin/accountchooser")
        || (url.contains("accounts.google.com") && url.contains("accountchooser"))
        || (url.contains("accounts.google.com/signin") && url.contains("flowname="))
        || (url.contains("accounts.google.com") && url.contains("prompt=select_account"))
    {
        return AuthPageType::GoogleAccountSelection;
    }
    if url.contains("accounts.google.com/saml") {
        return AuthPageType::GoogleSamlRedirect;
    }

    for pattern in AUTH_URL_PATTERNS {
        if url.contains(&pattern.to_lowercase()) {
            return AuthPageType::GenericOAuth;
        }
    }

    let has_sso_url = SSO_URL_PATTERNS.iter().any(|p| url.contains(&p.to_lowercase()));
    let has_redirect = SSO_REDIRECT_PARAMS.iter().any(|p| url.contains(p));
    if has_sso_url && has_redirect {
        return AuthPageType::CustomSso;
    }

    // If we're already at the target URL, skip content-based heuristics.
    // Pages at the target may contain Google widgets, "sign in" text, etc.
    // that would cause false positives.
    let at_target = match target_url {
        Some(target) => {
            let current = url.trim_end_matches('/');
            let target_norm = target.to_lowercase();
            let target_norm = target_norm.trim_end_matches('/');
            current == target_norm
        }
        None => false,
    };

    if !at_target {
        if let Ok(text) = page.text().await {
            let text_lower = text.to_lowercase();
            let has_sso_content = SSO_CONTENT_SIGNALS.iter().any(|s| text_lower.contains(s));
            if has_sso_content {
                if has_sso_url || has_redirect {
                    return AuthPageType::CustomSso;
                }
                let has_google_widget = text_lower.contains("accounts.google.com")
                    || text_lower.contains("googleapis.com/")
                    || text_lower.contains("g_id_onload")
                    || text_lower.contains("google-signin");
                if has_google_widget {
                    return AuthPageType::CustomSso;
                }
            }
        }
    }

    let was_redirected = !at_target;
    if was_redirected {
        if detect_auth_button(page).await.is_some() {
            return AuthPageType::GenericLogin;
        }
    }

    AuthPageType::NotAuth
}

/// Execute the appropriate click strategy for the detected auth page type.
/// Supports multi-step flows (e.g., account selection → consent) with a depth limit.
/// `preferred_account` can be a full email ("user@company.com") or domain ("@company.com").
/// Falls back to `PREFERRED_ACCOUNT` env var, then first available account.
pub fn click_authorize_with_account<'a>(
    page: &'a Page,
    auth_type: &'a AuthPageType,
    preferred_account: Option<&'a str>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = AuthResult> + Send + 'a>> {
    Box::pin(click_authorize_inner(page, auth_type, 0, preferred_account))
}

const AUTH_BUTTON_SCORE_JS: &str = r#"
    ((shouldClick) => {
        const keywords = KEYWORDS_PLACEHOLDER;
        const elements = document.querySelectorAll('a, button, [role="button"], input[type="submit"], span[onclick], div[onclick]');
        let bestEl = null;
        let bestText = null;
        let bestScore = 0;
        const NON_AUTH_HREF_PATTERNS = ['docs.google.com', 'drive.google.com', 'sheets.google.com', 'slides.google.com', 'calendar.google.com', 'mail.google.com'];
        for (const el of elements) {
            const text = (el.textContent || el.value || '').trim().toLowerCase();
            if (text.length === 0 || text.length > 100) continue;
            const rect = el.getBoundingClientRect();
            if (rect.width < 10 || rect.height < 10) continue;
            const href = (el.href || el.getAttribute('href') || '').toLowerCase();
            if (href && NON_AUTH_HREF_PATTERNS.some(p => href.includes(p))) continue;
            let score = 0;
            for (const kw of keywords) {
                if (text.includes(kw)) {
                    const isCJK = /[\u4e00-\u9fff]/.test(kw);
                    score += (kw.length > 3 || isCJK) ? 2 : 1;
                }
            }
            if (score > bestScore) {
                bestScore = score;
                bestEl = el;
                bestText = text.substring(0, 80);
            }
        }
        if (bestEl && bestScore >= 2) {
            if (shouldClick) {
                bestEl.click();
                return (bestEl.textContent || bestEl.value || '').trim().substring(0, 80);
            }
            return bestText;
        }
        return null;
    })(SHOULD_CLICK_PLACEHOLDER)
"#;

fn build_auth_button_score_js(should_click: bool) -> String {
    let keywords_json = serde_json::to_string(AUTH_BUTTON_KEYWORDS).unwrap_or_default();
    AUTH_BUTTON_SCORE_JS
        .replace("KEYWORDS_PLACEHOLDER", &keywords_json)
        .replace(
            "SHOULD_CLICK_PLACEHOLDER",
            if should_click { "true" } else { "false" },
        )
}

/// Detect if the page has any clickable element with auth/login keywords.
/// Returns the best candidate's text if found. Used for generic login detection.
/// Should only be called when URL-based redirect detection suggests this is a login page.
pub async fn detect_auth_button(page: &Page) -> Option<String> {
    let js_final = build_auth_button_score_js(false);

    match page.evaluate::<Option<String>>(&js_final).await {
        Ok(Some(text)) => {
            tracing::debug!(button_text = %text, "Detected auth button");
            Some(text)
        }
        _ => None,
    }
}

/// Find and click the best auth/login button on the page using keyword scoring.
/// More generic than try_sso_click; works on any login page (yapi, custom portals, etc.)
pub async fn click_best_auth_button(page: &Page) -> bool {
    let js_final = build_auth_button_score_js(true);

    match page.evaluate::<Option<String>>(&js_final).await {
        Ok(Some(text)) => {
            tracing::info!(button_text = %text, "Clicked auth button (generic)");
            true
        }
        Ok(None) => false,
        Err(e) => {
            tracing::debug!(error = %e, "Failed to find/click auth button");
            false
        }
    }
}

const MAX_AUTH_DEPTH: u8 = 5;

fn click_authorize_inner<'a>(
    page: &'a Page,
    auth_type: &'a AuthPageType,
    depth: u8,
    preferred_account: Option<&'a str>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = AuthResult> + Send + 'a>> {
    Box::pin(click_authorize_impl(page, auth_type, depth, preferred_account))
}

async fn click_authorize_impl(page: &Page, auth_type: &AuthPageType, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    if depth >= MAX_AUTH_DEPTH {
        return AuthResult {
            success: false,
            auth_type: auth_type.clone(),
            final_url: page.url().await.unwrap_or_default(),
            message: "Authorization flow exceeded maximum depth — may require manual intervention.".to_string(),
        };
    }

    let initial_url = page.url().await.unwrap_or_default();

    match auth_type {
        AuthPageType::GoogleOAuthConsent => {
            handle_google_oauth_consent(page, &initial_url, depth, preferred_account).await
        }
        AuthPageType::GoogleAccountSelection => {
            handle_google_account_selection(page, &initial_url, depth, preferred_account).await
        }
        AuthPageType::GoogleSamlRedirect => {
            handle_google_saml(page, &initial_url, depth, preferred_account).await
        }
        AuthPageType::GenericOAuth => {
            handle_generic_oauth(page, &initial_url, depth, preferred_account).await
        }
        AuthPageType::CustomSso => {
            handle_custom_sso(page, &initial_url, depth, preferred_account).await
        }
        AuthPageType::GenericLogin => {
            handle_generic_login(page, &initial_url, depth, preferred_account).await
        }
        AuthPageType::NotAuth => AuthResult {
            success: false,
            auth_type: AuthPageType::NotAuth,
            final_url: initial_url,
            message: "Page is not a recognized authorization page. No action taken.".to_string(),
        },
    }
}

async fn handle_google_oauth_consent(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    tracing::info!("Handling Google OAuth consent page");

    page.wait(1500).await;

    let clicked = try_click_sequence(page, &[
        ClickAction::Selector("#submit_approve_access"),
        ClickAction::Selector("button[id*='submit_approve']"),
        ClickAction::Text("Allow"),
        ClickAction::Text("允许"),
        ClickAction::Text("Continue"),
        ClickAction::Text("继续"),
        ClickAction::Selector("button[type='submit']"),
    ]).await;

    if !clicked {
        return AuthResult {
            success: false,
            auth_type: AuthPageType::GoogleOAuthConsent,
            final_url: page.url().await.unwrap_or_default(),
            message: "Could not find the authorization button on Google OAuth page.".to_string(),
        };
    }

    wait_for_auth_completion(page, initial_url, AuthPageType::GoogleOAuthConsent, depth, preferred_account).await
}

async fn handle_google_account_selection(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    tracing::info!(preferred = ?preferred_account, "Handling Google account selection page");

    page.wait(1500).await;

    let accounts = list_google_accounts(page).await;
    if accounts.is_empty() {
        return AuthResult {
            success: false,
            auth_type: AuthPageType::GoogleAccountSelection,
            final_url: page.url().await.unwrap_or_default(),
            message: "No accounts found on the selection page.".to_string(),
        };
    }

    tracing::info!(
        count = accounts.len(),
        accounts = ?accounts.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
        "Found Google accounts"
    );

    let target_idx = select_preferred_account(&accounts, preferred_account);
    let target_email = &accounts[target_idx].email;
    tracing::info!(selected = %target_email, index = target_idx, "Selecting account");

    let dom_index = accounts[target_idx].index;
    let clicked = click_account_by_email(page, target_email).await
        || click_account_by_index(page, dom_index).await;

    if clicked {
        tracing::info!(email = %target_email, "Clicked account");
        page.wait(2000).await;
        let _ = page.wait_for_network_idle(500, 8000).await;

        let new_url = page.url().await.unwrap_or_default();
        let new_auth_type = detect_auth_page(page).await;

        match new_auth_type {
            AuthPageType::GoogleOAuthConsent => {
                return click_authorize_inner(page, &AuthPageType::GoogleOAuthConsent, depth + 1, preferred_account).await;
            }
            AuthPageType::NotAuth => {
                return AuthResult {
                    success: true,
                    auth_type: AuthPageType::GoogleAccountSelection,
                    final_url: new_url,
                    message: format!("Account '{}' selected, authorization completed.", target_email),
                };
            }
            _ => {
                return wait_for_auth_completion(page, initial_url, AuthPageType::GoogleAccountSelection, depth, preferred_account).await;
            }
        }
    }

    AuthResult {
        success: false,
        auth_type: AuthPageType::GoogleAccountSelection,
        final_url: page.url().await.unwrap_or_default(),
        message: format!(
            "Could not click account. Available: [{}]",
            accounts.iter().map(|a| a.email.as_str()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Account info extracted from a Google account chooser page.
#[derive(Debug, Clone)]
pub struct GoogleAccount {
    pub email: String,
    pub index: usize,
}

/// List all Google accounts visible on an account chooser page.
pub async fn list_google_accounts(page: &Page) -> Vec<GoogleAccount> {
    let js = r#"
        (() => {
            const items = document.querySelectorAll('[data-identifier], [data-email], .JDAKTe');
            const accounts = [];
            for (let i = 0; i < items.length; i++) {
                const email = items[i].getAttribute('data-identifier')
                    || items[i].getAttribute('data-email')
                    || '';
                if (email) accounts.push({ email, index: i });
            }
            return accounts;
        })()
    "#;

    #[derive(serde::Deserialize)]
    struct RawAccount {
        email: String,
        index: usize,
    }

    match super::input::extract::<Vec<RawAccount>>(page, js).await {
        Ok(raw) => raw.into_iter().map(|r| GoogleAccount { email: r.email, index: r.index }).collect(),
        Err(_) => vec![],
    }
}

/// Determine which account to select based on preference.
///
/// Matching rules (in priority order):
/// 1. Exact email match (case-insensitive)
/// 2. Domain suffix match (e.g. "@company.com" matches "user@company.com")
/// 3. Fallback to first account (index 0)
fn select_preferred_account(accounts: &[GoogleAccount], preferred: Option<&str>) -> usize {
    let source;
    let pref = match preferred {
        Some(p) if !p.is_empty() => { source = "param"; p.to_lowercase() }
        _ => match std::env::var("PREFERRED_ACCOUNT").ok() {
            Some(env_val) if !env_val.is_empty() => { source = "env(PREFERRED_ACCOUNT)"; env_val.to_lowercase() }
            _ => {
                tracing::debug!(selected = %accounts[0].email, "No preference set, defaulting to first account");
                return 0;
            }
        }
    };

    tracing::info!(preference = %pref, source, "Resolving account preference");

    if let Some(idx) = find_matching_account(accounts, &pref) {
        tracing::info!(
            matched = %accounts[idx].email, index = idx,
            match_type = if pref.starts_with('@') { "domain_suffix" } else { "exact_email" },
            "Account preference matched"
        );
        return idx;
    }

    tracing::warn!(preferred = %pref, source, "No matching account found, falling back to first");
    0
}

fn find_matching_account(accounts: &[GoogleAccount], pattern: &str) -> Option<usize> {
    let pattern = pattern.to_lowercase();

    // Exact email match
    for (i, acct) in accounts.iter().enumerate() {
        if acct.email.to_lowercase() == pattern {
            return Some(i);
        }
    }

    // Domain suffix match (e.g. "@company.com")
    if pattern.starts_with('@') {
        for (i, acct) in accounts.iter().enumerate() {
            if acct.email.to_lowercase().ends_with(&pattern) {
                return Some(i);
            }
        }
    }

    None
}

async fn click_account_by_email(page: &Page, email: &str) -> bool {
    let selector = format!("[data-identifier='{}']", email);
    if page.try_click(&selector).await.unwrap_or(false) {
        return true;
    }
    let selector = format!("[data-email='{}']", email);
    page.try_click(&selector).await.unwrap_or(false)
}

async fn click_account_by_index(page: &Page, index: usize) -> bool {
    let js = format!(
        r#"(() => {{
            const items = document.querySelectorAll('[data-identifier], [data-email], .JDAKTe');
            if (items.length > {idx}) {{ items[{idx}].click(); return true; }}
            return false;
        }})()"#,
        idx = index
    );
    page.evaluate::<bool>(&js).await.unwrap_or(false)
}

async fn handle_google_saml(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    tracing::info!("Handling Google SAML redirect — waiting for redirect to complete");

    let _ = page.wait_for_network_idle(500, 10000).await;
    page.wait(2000).await;

    let new_auth_type = detect_auth_page(page).await;
    match new_auth_type {
        AuthPageType::NotAuth => AuthResult {
            success: true,
            auth_type: AuthPageType::GoogleSamlRedirect,
            final_url: page.url().await.unwrap_or_default(),
            message: "SAML redirect completed.".to_string(),
        },
        AuthPageType::GoogleOAuthConsent => {
            click_authorize_inner(page, &AuthPageType::GoogleOAuthConsent, depth + 1, preferred_account).await
        }
        AuthPageType::GoogleAccountSelection => {
            click_authorize_inner(page, &AuthPageType::GoogleAccountSelection, depth + 1, preferred_account).await
        }
        _ => {
            wait_for_auth_completion(page, initial_url, AuthPageType::GoogleSamlRedirect, depth, preferred_account).await
        }
    }
}

async fn handle_generic_oauth(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    tracing::info!("Handling generic OAuth consent page");

    page.wait(1500).await;

    let mut actions: Vec<ClickAction> = AUTH_CONSENT_BUTTON_TEXTS
        .iter()
        .map(|t| ClickAction::Text(t))
        .collect();
    actions.push(ClickAction::Selector("button[type='submit']"));
    actions.push(ClickAction::Selector("input[type='submit']"));

    let clicked = try_click_sequence(page, &actions).await;

    if !clicked {
        return AuthResult {
            success: false,
            auth_type: AuthPageType::GenericOAuth,
            final_url: page.url().await.unwrap_or_default(),
            message: "Could not find an authorization button on the OAuth page.".to_string(),
        };
    }

    wait_for_auth_completion(page, initial_url, AuthPageType::GenericOAuth, depth, preferred_account).await
}

async fn handle_custom_sso(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    tracing::info!(url = %initial_url, "Handling custom SSO page with embedded Google Sign-In");

    let target_url = extract_sso_target(initial_url);
    tracing::info!(target = ?target_url, "Extracted SSO redirect target");

    page.wait(2000).await;

    let clicked = try_click_sequence(page, &[
        ClickAction::Selector("a[href*='accounts.google.com']"),
        ClickAction::Selector("button[class*='google']"),
        ClickAction::Selector("a[class*='google']"),
        ClickAction::Selector("[id*='google-signin']"),
        ClickAction::Selector("[class*='g_id_signin']"),
        ClickAction::TextContains("sign in as"),
        ClickAction::Text("Sign in with Google"),
        ClickAction::Text("Login with Google"),
        ClickAction::Text("Continue with Google"),
        ClickAction::Text("Sign in"),
        ClickAction::Text("登录"),
        ClickAction::Selector("button[type='submit']"),
        ClickAction::Selector("input[type='submit']"),
    ]).await;

    let clicked = if clicked { true } else {
        try_google_signin_iframe(page).await
    };

    if !clicked {
        if let Some(ref target) = target_url {
            tracing::info!(target = %target, "No button found but trying direct navigation to target");
            if super::navigate(page, target, 15).await.is_ok() {
                let final_url = page.url().await.unwrap_or_default();
                let still_sso = final_url.to_lowercase().contains("auth.html")
                    || final_url.to_lowercase().contains("stlogin");
                return AuthResult {
                    success: !still_sso,
                    auth_type: AuthPageType::CustomSso,
                    final_url,
                    message: if still_sso {
                        "Direct navigation to target redirected back to SSO. Login required.".to_string()
                    } else {
                        "Navigated directly to target page.".to_string()
                    },
                };
            }
        }
        return AuthResult {
            success: false,
            auth_type: AuthPageType::CustomSso,
            final_url: page.url().await.unwrap_or_default(),
            message: "Could not find a sign-in button on the SSO page (checked main document and iframes).".to_string(),
        };
    }

    // Wait for SSO callback/redirect (One Tap flows use JS callbacks, may be slow)
    for i in 0..3 {
        page.wait(2000).await;
        let _ = page.wait_for_network_idle(500, 5000).await;

        let current_url = page.url().await.unwrap_or_default();
        if current_url != initial_url {
            tracing::info!(new_url = %current_url, "SSO redirected after click");
            break;
        }
        if i < 2 {
            tracing::debug!("Still on SSO page, waiting more...");
        }
    }

    let post_click_url = page.url().await.unwrap_or_default();

    if post_click_url == initial_url {
        if let Some(ref target) = target_url {
            tracing::info!(target = %target, "SSO page didn't redirect, trying direct navigation to target");
            if super::navigate(page, target, 15).await.is_ok() {
                let final_url = page.url().await.unwrap_or_default();
                let still_sso = final_url.to_lowercase().contains("auth.html")
                    || final_url.to_lowercase().contains("stlogin");
                return AuthResult {
                    success: !still_sso,
                    auth_type: AuthPageType::CustomSso,
                    final_url,
                    message: if still_sso {
                        "SSO click completed but session not established. Try sync_login.".to_string()
                    } else {
                        "SSO authorization completed, navigated to target page.".to_string()
                    },
                };
            }
        }
    }

    finish_sso_auth(page, &post_click_url, depth, preferred_account).await
}

/// Try to click sign-in buttons on a custom SSO page.
/// Returns true if something was clicked. Exported for server-level popup coordination.
pub async fn try_sso_click(page: &Page) -> bool {
    // 1. Try specific selectors (fast, precise)
    // Only match elements clearly intended for auth — no generic "google" class matching.
    let clicked = try_click_sequence(page, &[
        ClickAction::Selector("a[href*='accounts.google.com']"),
        ClickAction::Selector("button[class*='google']"),
        ClickAction::Selector("[id*='google-signin']"),
        ClickAction::Selector("[class*='g_id_signin']"),
        ClickAction::TextContains("sign in as"),
        ClickAction::Text("Sign in with Google"),
        ClickAction::Text("Login with Google"),
        ClickAction::Text("Continue with Google"),
    ]).await;
    if clicked { return true; }

    // 2. Try Google Sign-In iframe
    if try_google_signin_iframe(page).await { return true; }

    // 3. Fallback: generic auth button scoring (for non-Google SSO like "AKULAKU SSO 登录")
    if click_best_auth_button(page).await { return true; }

    // 4. Last resort: generic form submission
    try_click_sequence(page, &[
        ClickAction::Text("Sign in"),
        ClickAction::Text("登录"),
        ClickAction::Selector("button[type='submit']"),
        ClickAction::Selector("input[type='submit']"),
    ]).await
}

async fn handle_generic_login(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    tracing::info!(url = %initial_url, "Handling generic login page");

    page.wait(1000).await;

    let clicked = click_best_auth_button(page).await;
    if !clicked {
        return AuthResult {
            success: false,
            auth_type: AuthPageType::GenericLogin,
            final_url: page.url().await.unwrap_or_default(),
            message: "Could not find a login button on the page.".to_string(),
        };
    }

    // Wait for redirect/popup response
    for _ in 0..3 {
        page.wait(2000).await;
        let _ = page.wait_for_network_idle(500, 5000).await;
        let current_url = page.url().await.unwrap_or_default();
        if current_url != initial_url {
            break;
        }
    }

    let post_click_url = page.url().await.unwrap_or_default();
    if post_click_url != initial_url {
        let new_auth_type = detect_auth_page(page).await;
        match new_auth_type {
            AuthPageType::NotAuth => AuthResult {
                success: true,
                auth_type: AuthPageType::GenericLogin,
                final_url: post_click_url,
                message: "Login completed, redirected to target page.".to_string(),
            },
            _ => click_authorize_inner(page, &new_auth_type, depth + 1, preferred_account).await,
        }
    } else {
        AuthResult {
            success: false,
            auth_type: AuthPageType::GenericLogin,
            final_url: post_click_url,
            message: "Login button clicked but page did not redirect. May need popup handling at server level.".to_string(),
        }
    }
}

pub fn extract_sso_target(url: &str) -> Option<String> {
    for param in SSO_REDIRECT_PARAMS {
        if let Some(pos) = url.find(param) {
            let value_start = pos + param.len();
            let value_end = url[value_start..].find('&').map(|i| value_start + i).unwrap_or(url.len());
            let encoded = &url[value_start..value_end];
            if let Ok(decoded) = urlencoding::decode(encoded) {
                let target = decoded.to_string();
                if target.starts_with("http://") || target.starts_with("https://") {
                    return Some(target);
                }
            }
        }
    }
    None
}

async fn try_google_signin_iframe(page: &Page) -> bool {
    let frames = page.frames().await.unwrap_or_default();

    for frame_info in &frames {
        let frame_url = frame_info.url.to_lowercase();
        let is_gsi_frame = frame_url.contains("accounts.google.com/gsi")
            || frame_url.contains("accounts.google.com/o/oauth2")
            || frame_url.contains("accounts.google.com/signin")
            || frame_url.contains("accounts.google.com/_/signin");

        if !is_gsi_frame {
            continue;
        }

        tracing::info!(frame_url = %frame_info.url, "Found Google Sign-In iframe");

        let click_js = r#"
            (function() {
                // Try clicking any clickable element in the Google Sign-In iframe
                var selectors = [
                    '#credential_picker_container [role="link"]',
                    '[data-identifier]',
                    '.nsm7Bb-HzV7m-LgbsSe',
                    '#continue',
                    'button',
                    '[role="button"]',
                    'a'
                ];
                for (var i = 0; i < selectors.length; i++) {
                    var el = document.querySelector(selectors[i]);
                    if (el) { el.click(); return true; }
                }
                return false;
            })()
        "#;

        let frame_selector = format!("iframe[src*='{}']",
            if frame_url.contains("accounts.google.com/gsi") {
                "accounts.google.com/gsi"
            } else if frame_url.contains("accounts.google.com/o/oauth2") {
                "accounts.google.com/o/oauth2"
            } else {
                "accounts.google.com"
            }
        );

        match page.evaluate_in_frame(&frame_selector, click_js).await {
            Ok(clicked) => {
                let clicked_bool: bool = clicked;
                if clicked_bool {
                    tracing::info!("Clicked inside Google Sign-In iframe");
                    return true;
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "evaluate_in_frame failed for Google Sign-In iframe");
            }
        }
    }

    // Fallback: click on the iframe element itself (coordinate-based)
    let iframe_click_js = r#"
        (() => {
            const selectors = [
                'iframe[src*="accounts.google.com/gsi"]',
                'iframe[src*="accounts.google.com"]',
                '#credential_picker_iframe',
                'iframe[id*="gsi"]'
            ];
            for (const sel of selectors) {
                const frame = document.querySelector(sel);
                if (frame) {
                    const r = frame.getBoundingClientRect();
                    if (r.width > 0 && r.height > 0) {
                        return { x: r.x + r.width / 2, y: r.y + r.height / 2, selector: sel };
                    }
                }
            }
            return null;
        })()
    "#;

    #[derive(serde::Deserialize)]
    struct IframeRect { x: f64, y: f64, #[allow(dead_code)] selector: String }

    if let Ok(Some(rect)) = super::input::extract::<Option<IframeRect>>(page, iframe_click_js).await {
        tracing::info!(x = rect.x, y = rect.y, "Clicking Google Sign-In iframe center");
        page.click_at(rect.x, rect.y).await.ok();
        return true;
    }

    false
}

async fn finish_sso_auth(page: &Page, initial_url: &str, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    let new_url = page.url().await.unwrap_or_default();
    let new_auth_type = detect_auth_page(page).await;

    match new_auth_type {
        AuthPageType::NotAuth => {
            let url_lower = new_url.to_lowercase();
            let target_reached = !url_lower.contains("auth.html")
                && !url_lower.contains("/login")
                && !url_lower.contains("/sso/");
            AuthResult {
                success: target_reached || new_url != initial_url,
                auth_type: AuthPageType::CustomSso,
                final_url: new_url,
                message: if target_reached {
                    "SSO authorization completed, redirected to target page.".to_string()
                } else {
                    "SSO click performed but target page not reached. May need retry.".to_string()
                },
            }
        }
        AuthPageType::CustomSso if new_url == initial_url => {
            AuthResult {
                success: false,
                auth_type: AuthPageType::CustomSso,
                final_url: new_url,
                message: "SSO page did not redirect after authorization attempt. Session may already be active — try read_page directly, or use sync_login.".to_string(),
            }
        }
        AuthPageType::GoogleOAuthConsent
        | AuthPageType::GoogleAccountSelection
        | AuthPageType::GoogleSamlRedirect => {
            click_authorize_inner(page, &new_auth_type, depth + 1, preferred_account).await
        }
        other => {
            click_authorize_inner(page, &other, depth + 1, preferred_account).await
        }
    }
}

#[derive(Debug)]
enum ClickAction<'a> {
    Selector(&'a str),
    Text(&'a str),
    TextContains(&'a str),
}

async fn try_click_sequence(page: &Page, actions: &[ClickAction<'_>]) -> bool {
    for action in actions {
        let clicked = match action {
            ClickAction::Selector(sel) => page.try_click(sel).await.unwrap_or(false),
            ClickAction::Text(text) => page.try_click_by_text(text).await.unwrap_or(false),
            ClickAction::TextContains(text) => {
                super::click::click_by_text_contains(page, text).await
            }
        };
        if clicked {
            tracing::info!(action = ?action, "Clicked auth button");
            return true;
        }
    }
    false
}

async fn wait_for_auth_completion(page: &Page, initial_url: &str, auth_type: AuthPageType, depth: u8, preferred_account: Option<&str>) -> AuthResult {
    page.wait(2000).await;
    let _ = page.wait_for_network_idle(500, 10000).await;

    let final_url = page.url().await.unwrap_or_default();

    if final_url != initial_url {
        let new_auth_type = detect_auth_page(page).await;
        match new_auth_type {
            AuthPageType::NotAuth => {
                tracing::info!(final_url = %final_url, "Authorization completed, redirected to target");
                AuthResult {
                    success: true,
                    auth_type,
                    final_url,
                    message: "Authorization completed successfully.".to_string(),
                }
            }
            AuthPageType::GoogleOAuthConsent => {
                click_authorize_inner(page, &AuthPageType::GoogleOAuthConsent, depth + 1, preferred_account).await
            }
            AuthPageType::GoogleAccountSelection => {
                click_authorize_inner(page, &AuthPageType::GoogleAccountSelection, depth + 1, preferred_account).await
            }
            other @ (AuthPageType::GenericOAuth | AuthPageType::GoogleSamlRedirect | AuthPageType::CustomSso | AuthPageType::GenericLogin) => {
                click_authorize_inner(page, &other, depth + 1, preferred_account).await
            }
        }
    } else {
        tracing::warn!("URL did not change after clicking auth button");
        AuthResult {
            success: false,
            auth_type,
            final_url,
            message: "Authorization button was clicked but page did not redirect. May require manual intervention.".to_string(),
        }
    }
}

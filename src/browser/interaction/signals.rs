/// Unified page block detection signals.
/// Shared by the interaction layer (pre-extraction) and content extractor (post-extraction).

pub const CAPTCHA_KEYWORDS: &[&str] = &[
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
    "图片未转正",
    "正在验证",
    "403 forbidden",
];

pub const CAPTCHA_URL_SIGNALS: &[&str] = &[
    "/sorry/",
    "captcha",
    "challenge",
    "wappass.baidu.com",
    "/passport/",
    "/verify/",
];

pub const LOGIN_WALL_SIGNALS: &[&str] = &[
    "你的登录状态已失效",
    "请先登录",
    "login required",
    "sign in to continue",
    "请登录后查看",
    "登录后可查看",
    "没有知识存在的荒原",
];

pub const AUTH_URL_PATTERNS: &[&str] = &[
    "accounts.google.com/o/oauth2",
    "accounts.google.com/signin",
    "accounts.google.com/saml",
    "accounts.google.com/AccountChooser",
    "/oauth/authorize",
    "/oauth2/authorize",
    "login.microsoftonline.com",
];

pub const SSO_URL_PATTERNS: &[&str] = &[
    "/auth.html",
    "/sso/login",
    "/sso/auth",
    "/cas/login",
    "/saml/login",
    "/saml2/login",
    "/adfs/ls",
];

pub const SSO_REDIRECT_PARAMS: &[&str] = &[
    "ret=",
    "redirect=",
    "redirect_uri=",
    "return_to=",
    "next=",
    "target=",
    "service=",
];

pub const SSO_CONTENT_SIGNALS: &[&str] = &[
    "sign in as",
    "sign in with google",
    "sign in with microsoft",
    "login with google",
    "使用google登录",
    "使用 google 登录",
    "google 登录",
    "continue with google",
    "use another account",
];

pub const AUTH_CONSENT_BUTTON_TEXTS: &[&str] = &[
    "Allow",
    "允许",
    "Authorize",
    "授权",
    "Grant",
    "Accept",
    "同意",
    "确认授权",
];

/// Broad keywords for detecting auth/login buttons on any page.
/// Used by generic button detection to identify pages that need authorization.
pub const AUTH_BUTTON_KEYWORDS: &[&str] = &[
    "sso",
    "login",
    "log in",
    "sign in",
    "signin",
    "登录",
    "登入",
    "authorize",
    "授权",
    "continue with google",
    "continue with microsoft",
    "sign in with google",
    "login with google",
    "sign in with microsoft",
    "login with microsoft",
    "oauth",
    "saml",
];

/// Combined signals for content quality gating (used by content extractor).
/// Any match triggers quality = 0.0 regardless of category.
pub fn all_block_signals() -> impl Iterator<Item = &'static str> {
    CAPTCHA_KEYWORDS
        .iter()
        .chain(LOGIN_WALL_SIGNALS.iter())
        .copied()
}

use rs_trafilatura::{Options, extract_with_options};

pub struct ContentExtractor;

pub struct ExtractedContent {
    pub title: String,
    pub content: String,
    pub quality: f64,
}

impl ExtractedContent {
    pub fn is_low_quality(&self) -> bool {
        self.quality < 0.3 || (self.content.len() < 100 && !self.title.is_empty())
    }

    pub fn low_quality_reason(&self) -> Option<&'static str> {
        if self.quality == 0.0 {
            Some("CAPTCHA or verification page")
        } else if self.quality < 0.3 {
            Some("very low extraction quality")
        } else if self.content.len() < 100 && !self.title.is_empty() {
            Some("content too short (likely empty or error page)")
        } else {
            None
        }
    }
}

const CAPTCHA_CONTENT_SIGNALS: &[&str] = &[
    "百度安全验证", "安全验证", "请完成验证", "请完成下方验证",
    "网络不给力", "图片未转正", "正在验证",
    "unusual traffic", "are not a robot", "captcha",
    "verify you are human", "one last step",
    "人机验证", "异常流量",
    "access denied", "403 forbidden",
    "没有知识存在的荒原",
    "你的登录状态已失效",
    "请先登录",
    "login required",
    "sign in to continue",
    "请登录后查看",
    "登录后可查看",
];

impl ContentExtractor {
    pub fn extract(html: &str, _url: &str, max_length: usize) -> anyhow::Result<ExtractedContent> {
        let options = Options {
            output_markdown: true,
            favor_precision: true,
            include_tables: true,
            include_links: true,
            include_formatting: true,
            deduplicate: true,
            ..Options::default()
        };

        let result = extract_with_options(html, &options)
            .map_err(|e| anyhow::anyhow!("Content extraction failed: {:?}", e))?;

        let title = result.metadata.title.unwrap_or_default();
        let quality = result.extraction_quality;
        let mut content = result.content_text;

        let content_lower = content.to_lowercase();
        let title_lower = title.to_lowercase();
        for signal in CAPTCHA_CONTENT_SIGNALS {
            if content_lower.contains(signal) || title_lower.contains(signal) {
                return Ok(ExtractedContent {
                    title,
                    content: format!("[CAPTCHA] Page returned a verification challenge instead of content. Signal: \"{}\"", signal),
                    quality: 0.0,
                });
            }
        }

        if content.len() > max_length {
            let safe_end = content.floor_char_boundary(max_length);
            if let Some(pos) = content[..safe_end].rfind("\n\n") {
                content.truncate(pos);
            } else {
                content.truncate(safe_end);
            }
            content.push_str("\n\n(... content truncated)");
        }

        Ok(ExtractedContent { title, content, quality })
    }
}

pub fn strip_markdown_links(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut link_text = String::new();
            let mut found_close = false;
            for inner in chars.by_ref() {
                if inner == ']' {
                    found_close = true;
                    break;
                }
                link_text.push(inner);
            }
            if found_close && chars.peek() == Some(&'(') {
                chars.next();
                for inner in chars.by_ref() {
                    if inner == ')' {
                        break;
                    }
                }
                result.push_str(&link_text);
            } else {
                result.push('[');
                result.push_str(&link_text);
                if found_close {
                    result.push(']');
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

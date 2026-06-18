use dom_smoothie::{Config, Readability, TextMode};

pub struct ContentExtractor;

pub struct ExtractedContent {
    pub title: String,
    pub content: String,
}

impl ContentExtractor {
    pub fn extract(html: &str, url: &str, max_length: usize) -> anyhow::Result<ExtractedContent> {
        let config = Config {
            text_mode: TextMode::Markdown,
            ..Default::default()
        };

        let mut readability = Readability::new(html, Some(url), Some(config))
            .map_err(anyhow::Error::from)?;
        let article = readability.parse().map_err(anyhow::Error::from)?;

        let title = article.title;
        let mut content = article.text_content.to_string();

        content = clean_boilerplate(&content);

        if has_garbled_text(&content) {
            content = remove_garbled_lines(&content);
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

        Ok(ExtractedContent { title, content })
    }
}

fn clean_boilerplate(text: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;

    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"(?:版权所有|Copyright|©).*?(?:保留所有权利|All [Rr]ights [Rr]eserved|违者必究|未经许可不得转载)[^\n]*",
            r"(?:广告热线|客服热线|联系电话|商务合作|新闻热线|监督热线|投诉电话|举报电话)[：:\s]*[\d\s\-,，、/]+",
            r"(?:邮箱|Email)[：:\s]*\S+@\S+",
            r"[京沪粤浙苏闽鲁川渝湘鄂赣皖]ICP[备证]\d+号[^\n]*",
            r"(?:下载|安装|打开)(?:新浪财经|东方财富|同花顺|雪球|百度|每日经济新闻)\s*(?:APP|app|客户端)[^\n]*",
            r"(?:索取稿酬|撤下您的作品|不希望作品出现在本站)[^\n]*",
            r"(?:左滑更多数据|截图来自|数据来源|声明：)[^\n]*",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    });

    static MULTI_NEWLINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

    let mut result = text.to_string();
    for pattern in PATTERNS.iter() {
        result = pattern.replace_all(&result, "").to_string();
    }
    MULTI_NEWLINE.replace_all(&result, "\n\n").trim().to_string()
}

fn has_garbled_text(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let garbled: usize = text
        .chars()
        .collect::<Vec<_>>()
        .windows(4)
        .filter(|w| w.iter().all(|c| is_garbled_char(*c)))
        .count();
    let total = text.chars().count();
    total > 0 && garbled as f64 / total as f64 > 0.15
}

fn is_garbled_char(c: char) -> bool {
    if c.is_ascii_graphic() || c.is_ascii_whitespace() {
        return false;
    }
    if ('\u{4e00}'..='\u{9fff}').contains(&c)     // CJK
        || ('\u{3000}'..='\u{303f}').contains(&c)  // CJK symbols
        || ('\u{ff00}'..='\u{ffef}').contains(&c)  // fullwidth forms
        || ('\u{00a0}'..='\u{00ff}').contains(&c)  // Latin-1 supplement
    {
        return false;
    }
    true
}

fn remove_garbled_lines(text: &str) -> String {
    text.lines()
        .filter(|line| {
            let total = line.chars().count();
            if total < 5 {
                return true;
            }
            let garbled = line.chars().filter(|c| is_garbled_char(*c)).count();
            (garbled as f64 / total as f64) <= 0.3
        })
        .collect::<Vec<_>>()
        .join("\n")
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

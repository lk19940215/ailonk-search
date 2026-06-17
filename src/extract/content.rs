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

        if content.len() > max_length {
            // Find nearest char boundary at or before max_length (safe for multi-byte UTF-8)
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

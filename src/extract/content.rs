use rs_trafilatura::{Options, extract_with_options};

pub struct ContentExtractor;

pub struct ExtractedContent {
    pub title: String,
    pub content: String,
}

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
        let mut content = result.content_text;

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

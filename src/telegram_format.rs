#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedChunk {
    pub html: String,
    pub plain: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Rendered {
    html: String,
    plain: String,
}

impl Rendered {
    fn visible_units(&self) -> usize {
        self.plain.encode_utf16().count()
    }
}

pub fn format_chunks(markdown: &str, max_units: usize) -> Vec<FormattedChunk> {
    assert!(max_units > 0, "max_units must be greater than zero");
    let mut blocks = render_blocks(markdown, max_units);
    if blocks.is_empty() {
        blocks.push(Rendered {
            html: "（Codex 返回了空消息）".to_owned(),
            plain: "（Codex 返回了空消息）".to_owned(),
        });
    }

    let mut chunks = Vec::new();
    let mut current = Rendered::default();
    for block in blocks {
        let separator_units = usize::from(!current.plain.is_empty()) * 2;
        if !current.plain.is_empty()
            && current.visible_units() + separator_units + block.visible_units() > max_units
        {
            chunks.push(FormattedChunk {
                html: current.html,
                plain: current.plain,
            });
            current = Rendered::default();
        }
        if !current.plain.is_empty() {
            current.html.push_str("\n\n");
            current.plain.push_str("\n\n");
        }
        current.html.push_str(&block.html);
        current.plain.push_str(&block.plain);
    }
    if !current.plain.is_empty() {
        chunks.push(FormattedChunk {
            html: current.html,
            plain: current.plain,
        });
    }
    chunks
}

fn render_blocks(markdown: &str, max_units: usize) -> Vec<Rendered> {
    let normalized = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if let Some(language) = fence_language(trimmed) {
            let mut code_lines = Vec::new();
            index += 1;
            while index < lines.len() && !lines[index].trim_start().starts_with("```") {
                code_lines.push(lines[index]);
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            let code = code_lines.join("\n");
            for part in split_utf16(&code, max_units) {
                let language_attr = sanitize_language(language)
                    .map(|value| format!(" class=\"language-{value}\""))
                    .unwrap_or_default();
                blocks.push(Rendered {
                    html: format!(
                        "<pre><code{language_attr}>{}</code></pre>",
                        escape_html(&part)
                    ),
                    plain: part,
                });
            }
            continue;
        }

        if index + 1 < lines.len()
            && looks_like_table_row(line)
            && is_table_separator(lines[index + 1])
        {
            let headers = parse_table_row(line);
            index += 2;
            while index < lines.len() && looks_like_table_row(lines[index]) {
                let cells = parse_table_row(lines[index]);
                blocks.extend(render_table_row(&headers, &cells, max_units));
                index += 1;
            }
            continue;
        }

        if let Some((level, title)) = heading(line) {
            let marker = if level <= 2 { "▰" } else { "▸" };
            let rendered = render_inline(title);
            blocks.extend(ensure_size(
                Rendered {
                    html: format!("<b>{marker} {}</b>", rendered.html),
                    plain: format!("{marker} {}", rendered.plain),
                },
                max_units,
            ));
            index += 1;
            continue;
        }

        if is_horizontal_rule(trimmed) {
            blocks.push(Rendered {
                html: "────────────".to_owned(),
                plain: "────────────".to_owned(),
            });
            index += 1;
            continue;
        }

        if trimmed.starts_with('>') {
            let mut html_lines = Vec::new();
            let mut plain_lines = Vec::new();
            while index < lines.len() && lines[index].trim_start().starts_with('>') {
                let content = lines[index]
                    .trim_start()
                    .strip_prefix('>')
                    .unwrap_or_default()
                    .trim_start();
                let rendered = render_inline(content);
                html_lines.push(rendered.html);
                plain_lines.push(rendered.plain);
                index += 1;
            }
            blocks.extend(ensure_size(
                Rendered {
                    html: format!("<blockquote>{}</blockquote>", html_lines.join("\n")),
                    plain: plain_lines.join("\n"),
                },
                max_units,
            ));
            continue;
        }

        if list_item(line).is_some() {
            let mut html_lines = Vec::new();
            let mut plain_lines = Vec::new();
            while index < lines.len() {
                let Some(item) = list_item(lines[index]) else {
                    break;
                };
                let rendered = render_inline(item.content);
                let indent = "  ".repeat(item.depth);
                let marker = task_marker(item.content).unwrap_or(item.marker);
                let content_html = strip_task_prefix(&rendered.html);
                let content_plain = strip_task_prefix(&rendered.plain);
                html_lines.push(format!("{indent}{marker} {content_html}"));
                plain_lines.push(format!("{indent}{marker} {content_plain}"));
                index += 1;
            }
            blocks.extend(ensure_size(
                Rendered {
                    html: html_lines.join("\n"),
                    plain: plain_lines.join("\n"),
                },
                max_units,
            ));
            continue;
        }

        let mut html_lines = Vec::new();
        let mut plain_lines = Vec::new();
        while index < lines.len() && !lines[index].trim().is_empty() {
            if !html_lines.is_empty() && is_special_start(&lines, index) {
                break;
            }
            let rendered = render_inline(lines[index].trim());
            html_lines.push(rendered.html);
            plain_lines.push(rendered.plain);
            index += 1;
        }
        blocks.extend(ensure_size(
            Rendered {
                html: html_lines.join("\n"),
                plain: plain_lines.join("\n"),
            },
            max_units,
        ));
    }
    blocks
}

fn is_special_start(lines: &[&str], index: usize) -> bool {
    let line = lines[index];
    let trimmed = line.trim();
    fence_language(trimmed).is_some()
        || heading(line).is_some()
        || is_horizontal_rule(trimmed)
        || trimmed.starts_with('>')
        || list_item(line).is_some()
        || (index + 1 < lines.len()
            && looks_like_table_row(line)
            && is_table_separator(lines[index + 1]))
}

fn render_table_row(headers: &[String], cells: &[String], max_units: usize) -> Vec<Rendered> {
    if cells.is_empty() {
        return Vec::new();
    }
    let mut html_lines = Vec::new();
    let mut plain_lines = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        let header = headers.get(index).map(String::as_str).unwrap_or("字段");
        let value = render_inline(cell.trim());
        let label = escape_html(header.trim());
        if index == 0 {
            html_lines.push(format!("<b>{label}：</b>{}", value.html));
        } else {
            html_lines.push(format!("<b>{label}：</b> {}", value.html));
        }
        plain_lines.push(format!("{}：{}", header.trim(), value.plain));
    }
    ensure_size(
        Rendered {
            html: format!("<blockquote>{}</blockquote>", html_lines.join("\n")),
            plain: plain_lines.join("\n"),
        },
        max_units,
    )
}

fn ensure_size(rendered: Rendered, max_units: usize) -> Vec<Rendered> {
    if rendered.visible_units() <= max_units {
        return vec![rendered];
    }
    split_utf16(&rendered.plain, max_units)
        .into_iter()
        .map(|plain| Rendered {
            html: escape_html(&plain),
            plain,
        })
        .collect()
}

fn render_inline(input: &str) -> Rendered {
    let mut rendered = Rendered::default();
    let mut index = 0;
    while index < input.len() {
        let rest = &input[index..];

        if let Some(stripped) = rest.strip_prefix('\\')
            && let Some(character) = stripped.chars().next()
        {
            push_text(&mut rendered, &character.to_string());
            index += 1 + character.len_utf8();
            continue;
        }

        if let Some((content, consumed)) = enclosed(rest, "`", "`") {
            rendered
                .html
                .push_str(&format!("<code>{}</code>", escape_html(content)));
            rendered.plain.push_str(content);
            index += consumed;
            continue;
        }
        if let Some((content, consumed)) =
            enclosed(rest, "**", "**").or_else(|| enclosed(rest, "__", "__"))
        {
            let inner = render_inline(content);
            rendered.html.push_str(&format!("<b>{}</b>", inner.html));
            rendered.plain.push_str(&inner.plain);
            index += consumed;
            continue;
        }
        if let Some((content, consumed)) = enclosed(rest, "~~", "~~") {
            let inner = render_inline(content);
            rendered.html.push_str(&format!("<s>{}</s>", inner.html));
            rendered.plain.push_str(&inner.plain);
            index += consumed;
            continue;
        }
        if rest.starts_with('[')
            && let Some(close_label) = rest.find("](")
            && let Some(close_url) = rest[close_label + 2..].find(')')
        {
            let label = &rest[1..close_label];
            let url_start = close_label + 2;
            let url = &rest[url_start..url_start + close_url];
            let label_rendered = render_inline(label);
            if is_safe_link(url) {
                rendered.html.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    escape_attribute(url),
                    label_rendered.html
                ));
                rendered.plain.push_str(&label_rendered.plain);
            } else {
                rendered.html.push_str(&format!(
                    "{} <code>{}</code>",
                    label_rendered.html,
                    escape_html(url)
                ));
                rendered
                    .plain
                    .push_str(&format!("{} ({url})", label_rendered.plain));
            }
            index += url_start + close_url + 1;
            continue;
        }
        if let Some((content, consumed)) = enclosed(rest, "*", "*") {
            let inner = render_inline(content);
            rendered.html.push_str(&format!("<i>{}</i>", inner.html));
            rendered.plain.push_str(&inner.plain);
            index += consumed;
            continue;
        }

        let character = rest.chars().next().expect("non-empty string");
        push_text(&mut rendered, &character.to_string());
        index += character.len_utf8();
    }
    rendered
}

fn push_text(rendered: &mut Rendered, text: &str) {
    rendered.html.push_str(&escape_html(text));
    rendered.plain.push_str(text);
}

fn enclosed<'a>(input: &'a str, open: &str, close: &str) -> Option<(&'a str, usize)> {
    let after_open = input.strip_prefix(open)?;
    if after_open.is_empty() {
        return None;
    }
    let end = after_open.find(close)?;
    if end == 0 {
        return None;
    }
    Some((&after_open[..end], open.len() + end + close.len()))
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(input: &str) -> String {
    escape_html(input).replace('"', "&quot;")
}

fn is_safe_link(url: &str) -> bool {
    ["https://", "http://", "tg://", "mailto:"]
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

fn fence_language(line: &str) -> Option<&str> {
    line.strip_prefix("```").map(str::trim)
}

fn sanitize_language(language: &str) -> Option<&str> {
    (!language.is_empty()
        && language
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'_')))
    .then_some(language)
}

fn heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&level) || trimmed.as_bytes().get(level) != Some(&b' ') {
        return None;
    }
    Some((level, trimmed[level + 1..].trim()))
}

fn is_horizontal_rule(line: &str) -> bool {
    let compact = line.chars().filter(|character| !character.is_whitespace());
    let value = compact.collect::<String>();
    value.len() >= 3
        && (value.chars().all(|character| character == '-')
            || value.chars().all(|character| character == '*'))
}

#[derive(Clone, Copy)]
struct ListItem<'a> {
    depth: usize,
    marker: &'a str,
    content: &'a str,
}

fn list_item(line: &str) -> Option<ListItem<'_>> {
    let leading = line.len() - line.trim_start().len();
    let depth = leading / 2;
    let trimmed = line.trim_start();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(content) = trimmed.strip_prefix(prefix) {
            return Some(ListItem {
                depth,
                marker: if depth == 0 { "•" } else { "◦" },
                content,
            });
        }
    }
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let suffix = &trimmed[digits..];
        if let Some(content) = suffix
            .strip_prefix(". ")
            .or_else(|| suffix.strip_prefix(") "))
        {
            return Some(ListItem {
                depth,
                marker: &trimmed[..digits + 1],
                content,
            });
        }
    }
    None
}

fn task_marker(content: &str) -> Option<&'static str> {
    let prefix = content.get(..3)?;
    if prefix.eq_ignore_ascii_case("[x]") {
        Some("☑")
    } else if prefix == "[ ]" {
        Some("☐")
    } else {
        None
    }
}

fn strip_task_prefix(content: &str) -> &str {
    content
        .strip_prefix("[x] ")
        .or_else(|| content.strip_prefix("[X] "))
        .or_else(|| content.strip_prefix("[ ] "))
        .unwrap_or(content)
}

fn looks_like_table_row(line: &str) -> bool {
    line.contains('|') && parse_table_row(line).len() >= 2
}

fn is_table_separator(line: &str) -> bool {
    let cells = parse_table_row(line);
    cells.len() >= 2
        && cells.iter().all(|cell| {
            let value = cell.trim();
            value.contains('-')
                && value
                    .chars()
                    .all(|character| character == '-' || character == ':')
        })
}

fn parse_table_row(line: &str) -> Vec<String> {
    let mut value = line.trim();
    if let Some(stripped) = value.strip_prefix('|') {
        value = stripped;
    }
    if let Some(stripped) = value.strip_suffix('|') {
        value = stripped;
    }
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '|' {
            cells.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(character);
        }
    }
    if escaped {
        current.push('\\');
    }
    cells.push(current.trim().to_owned());
    cells
}

fn split_utf16(input: &str, max_units: usize) -> Vec<String> {
    if input.is_empty() {
        return vec![String::new()];
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut units = 0;
    for character in input.chars() {
        let character_units = character.len_utf16();
        if units > 0 && units + character_units > max_units {
            parts.push(current);
            current = String::new();
            units = 0;
        }
        current.push(character);
        units += character_units;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_telegram_safe_rich_text() {
        let chunks = format_chunks(
            "## 状态\n\n**服务**：`term-pigeon` <active>\n\n- 正常\n- [x] 已验证\n\n[文档](https://example.com?a=1&b=2)",
            3800,
        );
        assert_eq!(chunks.len(), 1);
        let html = &chunks[0].html;
        assert!(html.contains("<b>▰ 状态</b>"));
        assert!(html.contains("<b>服务</b>：<code>term-pigeon</code> &lt;active&gt;"));
        assert!(html.contains("• 正常\n☑ 已验证"));
        assert!(html.contains("<a href=\"https://example.com?a=1&amp;b=2\">文档</a>"));
    }

    #[test]
    fn renders_code_blocks_and_escapes_html() {
        let chunks = format_chunks("```bash\necho '<ok>' && true\n```", 3800);
        assert_eq!(
            chunks[0].html,
            "<pre><code class=\"language-bash\">echo '&lt;ok&gt;' &amp;&amp; true</code></pre>"
        );
    }

    #[test]
    fn converts_markdown_tables_to_mobile_friendly_rows() {
        let chunks = format_chunks(
            "| 服务 | 状态 | 用途 |\n|---|---|---|\n| `term-pigeon` | **active** | Telegram |",
            3800,
        );
        let html = &chunks[0].html;
        assert!(html.contains("<blockquote>"));
        assert!(html.contains("<b>服务：</b><code>term-pigeon</code>"));
        assert!(html.contains("<b>状态：</b> <b>active</b>"));
        assert!(!html.contains('|'));
    }

    #[test]
    fn local_file_links_become_readable_code_paths() {
        let chunks = format_chunks("配置见 [service](/etc/systemd/system/or.service:1)", 3800);
        assert!(
            chunks[0]
                .html
                .contains("service <code>/etc/systemd/system/or.service:1</code>")
        );
    }

    #[test]
    fn splits_by_telegram_visible_utf16_limit() {
        let input = format!("# 标题\n\n{}", "😀".repeat(25));
        let chunks = format_chunks(&input, 20);
        assert!(chunks.len() >= 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.plain.encode_utf16().count() <= 20)
        );
    }
}

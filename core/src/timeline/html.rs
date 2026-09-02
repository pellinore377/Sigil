//! Matrix `formatted_body` → the HTML subset Qt's RichText understands. Order matters:
//! ruma's spec sanitizer runs first, and only then does the presentation pass rewrite
//! what survives. Link colour is not baked in — the view themes it.

use ruma_html::{sanitize_html, HtmlSanitizerMode, RemoveReplyFallback};

/// Tags Qt's rich text renders that we are willing to emit.
const KEEP: &[&str] = &[
    "b", "strong", "i", "em", "u", "s", "del", "code", "pre", "blockquote", "a",
    "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6", "br", "p", "hr",
    "table", "tr", "td", "th", "sub", "sup", "span", "font", "div",
];

/// Schemes a link may use. Anything else loses its anchor and renders as text.
const SCHEMES: &[&str] = &["http://", "https://", "mailto:", "matrix:"];

pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Plain body → rich text: escape, wrap bare URLs in anchors, keep newlines.
pub fn linkify(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 16);
    let mut rest = body;
    while let Some(start) = find_url(rest) {
        out.push_str(&escape(&rest[..start]));
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '<')
            .unwrap_or(tail.len());
        let mut url = &tail[..end];
        // Trailing punctuation is almost never part of the link.
        while let Some(last) = url.chars().last() {
            if ".,;:!?)]}'\"".contains(last) { url = &url[..url.len() - last.len_utf8()]; } else { break }
        }
        if url.is_empty() { out.push_str(&escape(&tail[..end])); }
        else { out.push_str(&format!("<a href=\"{0}\">{0}</a>", escape(url))); }
        rest = &tail[url.len().max(1).min(tail.len())..];
    }
    out.push_str(&escape(rest));
    out.replace('\n', "<br>")
}

fn find_url(s: &str) -> Option<usize> {
    let h = s.find("http://");
    let s2 = s.find("https://");
    match (h, s2) { (Some(a), Some(b)) => Some(a.min(b)), (Some(a), None) => Some(a), (None, b) => b }
}

/// `formatted_body` → Qt rich text.
pub fn to_rich_text(html: &str) -> String {
    let safe = sanitize_html(html, HtmlSanitizerMode::Strict, RemoveReplyFallback::Yes);
    present(&safe)
}

/// Sanitized Matrix HTML → the subset Qt renders; the tag allow-list is a second line of defence.
fn present(safe: &str) -> String {
    let b = safe.as_bytes();
    let mut out = String::with_capacity(safe.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'<' { out.push(b[i] as char); i += 1; continue; }
        let Some(close) = safe[i..].find('>').map(|p| i + p) else {
            out.push_str("&lt;");
            i += 1;
            continue;
        };
        let raw = &safe[i + 1..close];
        i = close + 1;

        let closing = raw.starts_with('/');
        let body = if closing { &raw[1..] } else { raw };
        let name: String = body
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if !KEEP.contains(&name.as_str()) { continue; }
        let attrs = &body[name.len()..];

        if closing {
            match name.as_str() {
                "span" | "font" => out.push_str("</font>"),
                "a" => out.push_str("</a>"),
                other => { out.push('<'); out.push('/'); out.push_str(other); out.push('>'); }
            }
            continue;
        }

        // Taken whole: `present()` would drop `class="language-…"` and re-escape syntect's spans.
        if name == "pre" && !closing {
            if let Some((inner, lang, after)) = take_pre(safe, i) {
                // No background: the bubble draws code as its own block (`to_parts`).
                out.push_str("<pre style=\"font-family:monospace; white-space:pre-wrap\">");
                out.push_str(&super::code::highlight(&unescape(&inner), lang.as_deref()));
                out.push_str("</pre>");
                i = after;
                continue;
            }
        }

        match name.as_str() {
            "a" => match attr(attrs, "href").filter(|h| SCHEMES.iter().any(|s| h.to_ascii_lowercase().starts_with(s))) {
                // No inline colour: the view themes links via Text.linkColor.
                Some(href) => out.push_str(&format!("<a href=\"{}\">", escape(&href))),
                None => out.push_str("<u>"),
            },
            "pre" => out.push_str("<pre style=\"font-family:monospace; white-space:pre-wrap\">"),
            "code" => out.push_str("<code style=\"font-family:monospace\">"),
            "span" | "font" => {
                if attrs.to_ascii_lowercase().contains("data-mx-spoiler") {
                    out.push_str("<font color=\"#888888\">[spoiler] ");
                } else {
                    match attr(attrs, "data-mx-color").or_else(|| attr(attrs, "color")).filter(|c| is_hex_colour(c)) {
                        Some(c) => out.push_str(&format!("<font color=\"{c}\">")),
                        None => out.push_str("<font>"),
                    }
                }
            }
            other => { out.push('<'); out.push_str(other); out.push('>'); }
        }
    }
    out
}

/// A message split into the pieces a bubble draws separately: a code block is laid out
/// full width with the surrounding text as captions. `None` when there is no code block.
pub fn to_parts(html: &str) -> Option<Vec<serde_json::Value>> {
    let safe = sanitize_html(html, HtmlSanitizerMode::Strict, RemoveReplyFallback::Yes);
    if !safe.to_ascii_lowercase().contains("<pre") { return None }

    let mut parts: Vec<serde_json::Value> = Vec::new();
    let mut rest = safe.as_str();
    let mut cut = 0usize;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(open) = lower[cut..].find("<pre").map(|p| cut + p) else { break };
        let Some(gt) = rest[open..].find('>').map(|p| open + p + 1) else { break };
        let Some((inner, lang, after)) = take_pre(rest, gt) else { break };

        push_text(&mut parts, &rest[..open]);
        parts.push(serde_json::json!({
            "t": "code",
            "lang": lang.clone().unwrap_or_default(),
            "html": super::code::highlight(&unescape(&inner), lang.as_deref()),
        }));
        rest = &rest[after..];
        cut = 0;
    }
    push_text(&mut parts, rest);
    if parts.iter().any(|p| p["t"] == "code") { Some(parts) } else { None }
}

fn push_text(parts: &mut Vec<serde_json::Value>, raw: &str) {
    let rendered = present(raw);
    if to_plain(&rendered).trim().is_empty() { return }
    parts.push(serde_json::json!({ "t": "text", "html": rendered }));
}

/// `(text, language, index after `</pre>`)`; the language comes off the inner
/// `<code class="language-…">`. Inner tags are stripped — Matrix wraps the body in `<code>`.
fn take_pre(safe: &str, from: usize) -> Option<(String, Option<String>, usize)> {
    let rest = &safe[from..];
    let end = rest.to_ascii_lowercase().find("</pre>")?;
    let inner_raw = &rest[..end];

    let mut lang = None;
    let mut text = String::with_capacity(inner_raw.len());
    let b = inner_raw.as_bytes();
    let mut j = 0usize;
    while j < b.len() {
        if b[j] != b'<' { text.push(b[j] as char); j += 1; continue }
        let Some(close) = inner_raw[j..].find('>').map(|p| j + p) else { break };
        let raw = &inner_raw[j + 1..close];
        if lang.is_none() && raw.to_ascii_lowercase().starts_with("code") {
            lang = attr(raw, "class")
                .and_then(|c| {
                    c.split_whitespace()
                        .find_map(|w| w.strip_prefix("language-").map(str::to_string))
                });
        }
        j = close + 1;
    }
    Some((text, lang, from + end + "</pre>".len()))
}

/// Entities back to characters so the highlighter sees source; it re-escapes on the way out.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        // Last, or an `&amp;lt;` in the source would come back as `<`.
        .replace("&amp;", "&")
}

/// Pull one attribute value out of a tag's attribute text.
fn attr(attrs: &str, key: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0usize;
    while let Some(pos) = lower[from..].find(key) {
        let at = from + pos;
        let after = &attrs[at + key.len()..];
        let trimmed = after.trim_start();
        if let Some(rest) = trimmed.strip_prefix('=') {
            let rest = rest.trim_start();
            let (quote, rest) = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => (q, &rest[1..]),
                _ => ('\0', rest),
            };
            let end = if quote == '\0' {
                rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len())
            } else {
                rest.find(quote).unwrap_or(rest.len())
            };
            return Some(rest[..end].to_string());
        }
        from = at + key.len();
    }
    None
}

fn is_hex_colour(s: &str) -> bool {
    let s = s.trim();
    (s.len() == 7 || s.len() == 4) && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Strip every tag, for previews and notifications.
pub fn to_plain(html: &str) -> String {
    let safe = sanitize_html(html, HtmlSanitizerMode::Strict, RemoveReplyFallback::Yes);
    let mut out = String::with_capacity(safe.len());
    let mut depth = 0;
    for c in safe.chars() {
        match c {
            '<' => depth += 1,
            '>' => { if depth > 0 { depth -= 1 } }
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_scripts_and_handlers() {
        let out = to_rich_text("<script>alert(1)</script><b onclick=\"steal()\">hi</b>");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.to_ascii_lowercase().contains("onclick"), "{out}");
        assert!(out.contains("hi"));
    }

    #[test]
    fn drops_javascript_urls() {
        let out = to_rich_text("<a href=\"javascript:alert(1)\">click</a>");
        assert!(!out.to_ascii_lowercase().contains("javascript"), "{out}");
        assert!(out.contains("click"));
    }

    #[test]
    fn drops_style_attributes_and_images() {
        let out = to_rich_text("<span style=\"background-image:url(https://tracker/x.png)\">x</span><img src=\"https://tracker/y.png\">");
        assert!(!out.contains("tracker"), "{out}");
        assert!(!out.contains("background-image"), "{out}");
    }

    #[test]
    fn removes_reply_fallback() {
        let out = to_rich_text("<mx-reply><blockquote>quoted</blockquote></mx-reply>real reply");
        assert!(!out.contains("quoted"), "{out}");
        assert!(out.contains("real reply"));
    }

    #[test]
    fn keeps_safe_links_without_baking_colour() {
        let out = to_rich_text("<a href=\"https://example.org/a\">site</a>");
        assert!(out.contains("href=\"https://example.org/a\""), "{out}");
        assert!(!out.to_ascii_lowercase().contains("color"), "{out}");
    }

    #[test]
    fn keeps_spec_colour() {
        let out = to_rich_text("<span data-mx-color=\"#ff0000\">red</span>");
        assert!(out.contains("#ff0000"), "{out}");
        let bad = to_rich_text("<span data-mx-color=\"expression(evil)\">x</span>");
        assert!(!bad.contains("evil"), "{bad}");
    }

    #[test]
    fn linkify_escapes_and_wraps() {
        let out = linkify("see https://example.org/a, and <b>not bold</b>");
        assert!(out.contains("<a href=\"https://example.org/a\">"), "{out}");
        assert!(out.contains("&lt;b&gt;"), "{out}");
        assert!(!out.contains("a,</a>"), "{out}");
    }

    #[test]
    fn plain_strips_markup() {
        assert_eq!(to_plain("<b>hi</b> there"), "hi there");
    }

    #[test]
    fn a_fenced_rust_block_comes_out_coloured() {
        let html = "<pre><code class=\"language-rust\">fn main() { let x = 1; }</code></pre>";
        let out = to_rich_text(html);
        assert!(out.contains("<pre"), "still a pre: {out}");
        assert!(out.contains("<span style=\"color:#"), "coloured: {out}");
        assert!(out.contains("main"), "the code survives: {out}");
    }

    #[test]
    fn a_block_with_no_language_is_plain_but_intact() {
        let out = to_rich_text("<pre><code>ls -la | grep x</code></pre>");
        assert!(out.contains("<pre"));
        assert!(!out.contains("<span style=\"color:#"), "nothing to colour: {out}");
        assert!(out.contains("ls -la"), "text kept: {out}");
    }

    #[test]
    fn source_inside_a_block_cannot_escape_as_markup() {
        // A code sample containing markup must come back as text, not as tags.
        let html = "<pre><code class=\"language-html\">&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>";
        let out = to_rich_text(html);
        assert!(!out.contains("<script"), "no live script: {out}");
        // The highlighter splits the sample across spans, so check the pieces.
        assert!(out.contains("&lt;") && out.contains("&gt;"), "escaped: {out}");
        assert!(out.contains(">script<"), "shown as text, not markup: {out}");
    }

    #[test]
    fn text_around_a_block_is_still_processed_normally() {
        let out = to_rich_text("<b>before</b><pre><code class=\"language-rust\">let y = 2;</code></pre><i>after</i>");
        assert!(out.contains("<b>before</b>"), "{out}");
        assert!(out.contains("<i>after</i>"), "{out}");
    }

    #[test]
    fn an_unterminated_block_does_not_swallow_the_message() {
        let out = to_rich_text("<pre><code>oops");
        assert!(out.contains("oops"), "{out}");
    }
}

//! HTML parser written from scratch (no external crates).
//!
//! Supports a practical subset of HTML5:
//! - start / end / self-closing tags, attributes (quoted & unquoted)
//! - comments, doctype, raw-text elements (`<style>`, `<script>`, `<title>`)
//! - void elements (`<br>`, `<img>`, `<hr>`, `<meta>`, `<link>`, `<input>`)
//! - character references (`&amp;`, `&lt;`, `&#39;`, `&nbsp;`, ...)
//! - implied end tags for `<p>` and `<li>` (the most common cases)
//!
//! The parser builds a simple DOM tree ([`Element`] / [`Node`]) that the
//! style and layout engines consume. It is intentionally forgiving:
//! unknown tags are kept, unclosed tags are closed at EOF, stray end tags
//! are ignored — just like a real browser.

use std::collections::HashMap;

/// A node in the DOM tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

/// An element node: tag name, attributes and children.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Lower-cased tag name, e.g. `"div"`, `"a"`, `"h1"`.
    pub tag: String,
    /// Lower-cased attribute names -> values.
    pub attributes: HashMap<String, String>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_ascii_lowercase(),
            attributes: HashMap::new(),
            children: Vec::new(),
        }
    }

    /// Attribute value by (case-insensitive) name.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    /// Whitespace-separated `class` attribute as a list.
    pub fn classes(&self) -> Vec<String> {
        self.attr("class")
            .map(|c| c.split_whitespace().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }

    /// Concatenated text of all descendant text nodes.
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        fn walk(node: &Node, out: &mut String) {
            match node {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => {
                    for c in &e.children {
                        walk(c, out);
                    }
                }
            }
        }
        for c in &self.children {
            walk(c, &mut out);
        }
        out
    }

    /// First descendant (including self) with the given tag, depth-first.
    pub fn find_first(&self, tag: &str) -> Option<&Element> {
        if self.tag == tag {
            return Some(self);
        }
        for c in &self.children {
            if let Node::Element(e) = c {
                if let Some(found) = e.find_first(tag) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// All descendants (including self) with the given tag, in document order.
    pub fn find_all(&self, tag: &str) -> Vec<&Element> {
        let mut out = Vec::new();
        fn walk<'a>(e: &'a Element, tag: &str, out: &mut Vec<&'a Element>) {
            if e.tag == tag {
                out.push(e);
            }
            for c in &e.children {
                if let Node::Element(child) = c {
                    walk(child, tag, out);
                }
            }
        }
        walk(self, tag, &mut out);
        out
    }

    /// True for elements that never have children or an end tag.
    pub fn is_void(&self) -> bool {
        is_void_tag(&self.tag)
    }
}

/// A parsed HTML document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The `<html>` element (synthesised if the source has none).
    pub root: Element,
    /// Text of `<title>`, or empty string.
    pub title: String,
}

impl Document {
    /// The `<body>` element, if present.
    pub fn body(&self) -> Option<&Element> {
        self.root.find_first("body")
    }

    /// All `<style>` contents concatenated (author styles).
    pub fn style_texts(&self) -> Vec<String> {
        self.root
            .find_all("style")
            .iter()
            .map(|e| e.text_content())
            .collect()
    }

    /// `href`s of `<link rel="stylesheet">` in document order.
    /// `src` of every `<img>` in document order.
    pub fn image_srcs(&self) -> Vec<String> {
        self.root
            .find_all("img")
            .into_iter()
            .filter_map(|e| e.attr("src").map(|s| s.to_string()))
            .collect()
    }

    pub fn stylesheet_hrefs(&self) -> Vec<String> {
        self.root
            .find_all("link")
            .into_iter()
            .filter(|e| {
                e.attr("rel")
                    .map(|r| r.to_ascii_lowercase().contains("stylesheet"))
                    .unwrap_or(false)
            })
            .filter_map(|e| e.attr("href").map(|s| s.to_string()))
            .collect()
    }
}

pub fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// Elements whose content is raw text (no tag parsing inside).
fn is_raw_text_tag(tag: &str) -> bool {
    matches!(tag, "style" | "script" | "title" | "textarea")
}

/// Block-level tags that implicitly close an open `<p>`.
fn closes_p(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "div"
            | "dl"
            | "fieldset"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
    )
}

enum Token {
    StartTag {
        tag: String,
        attrs: HashMap<String, String>,
        self_closing: bool,
    },
    EndTag {
        tag: String,
    },
    Text(String),
}

/// Parse an HTML string into a [`Document`]. Never fails: on malformed
/// input it produces a best-effort tree.
pub fn parse(html: &str) -> Document {
    let tokens = tokenize(html);
    let mut stack: Vec<Element> = vec![Element::new("html")];
    let mut saw_html = false;
    let mut saw_body = false;

    for token in tokens {
        match token {
            Token::Text(text) => {
                if text.trim().is_empty() && stack.len() == 1 {
                    continue; // whitespace outside <html>
                }
                stack.last_mut().unwrap().children.push(Node::Text(text));
            }
            Token::StartTag {
                tag,
                attrs,
                self_closing,
            } => {
                if tag == "html" {
                    if saw_html {
                        continue;
                    }
                    saw_html = true;
                    stack.last_mut().unwrap().attributes.extend(attrs);
                    continue;
                }
                if tag == "body" {
                    saw_body = true;
                }
                // Implied end tags.
                if closes_p(&tag) {
                    close_open(&mut stack, "p");
                }
                if tag == "li" {
                    close_open(&mut stack, "li");
                }
                if tag == "head" || tag == "body" {
                    // Close an open <head> when <body> starts.
                    if tag == "body" {
                        close_open(&mut stack, "head");
                    }
                }

                let mut el = Element::new(&tag);
                el.attributes = attrs;
                if self_closing || is_void_tag(&tag) {
                    stack.last_mut().unwrap().children.push(Node::Element(el));
                } else {
                    stack.push(el);
                }
            }
            Token::EndTag { tag } => {
                if tag == "html" || tag == "body" {
                    // Keep the synthetic root; just unwind to it.
                    while stack.len() > 1 {
                        let done = stack.pop().unwrap();
                        stack.last_mut().unwrap().children.push(Node::Element(done));
                    }
                    continue;
                }
                close_open(&mut stack, &tag);
            }
        }
    }

    // Unwind everything still open.
    while stack.len() > 1 {
        let done = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Node::Element(done));
    }
    let root = stack.pop().unwrap();

    // If the source had no <body>, browsers still render content — wrap
    // top-level non-head children into a synthetic body so layout has a
    // single block container to work with.
    let root = ensure_body(root, saw_body);

    let title = root
        .find_first("title")
        .map(|t| t.text_content().trim().to_string())
        .unwrap_or_default();

    Document { root, title }
}

fn ensure_body(mut root: Element, saw_body: bool) -> Element {
    if saw_body || root.find_first("body").is_some() {
        return root;
    }
    let mut body = Element::new("body");
    let mut rest = Vec::new();
    for child in root.children.drain(..) {
        match &child {
            Node::Element(e) if e.tag == "head" || e.tag == "title" => rest.push(child),
            _ => body.children.push(child),
        }
    }
    root.children = rest;
    root.children.push(Node::Element(body));
    root
}

/// Pop the stack up to and including the nearest open `tag`, attaching
/// popped elements to their parents. No-op if `tag` is not open.
fn close_open(stack: &mut Vec<Element>, tag: &str) {
    if let Some(pos) = stack.iter().rposition(|e| e.tag == tag) {
        while stack.len() > pos {
            let done = stack.pop().unwrap();
            if let Some(parent) = stack.last_mut() {
                parent.children.push(Node::Element(done));
            }
        }
    }
}

fn tokenize(html: &str) -> Vec<Token> {
    let bytes = html.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let n = bytes.len();

    while i < n {
        if bytes[i] == b'<' {
            // Comment.
            if html[i..].starts_with("<!--") {
                match html[i + 4..].find("-->") {
                    Some(end) => i += 4 + end + 3,
                    None => break,
                }
                continue;
            }
            // Doctype / declaration / processing instruction.
            if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
                match html[i..].find('>') {
                    Some(end) => i += end + 1,
                    None => break,
                }
                continue;
            }
            // End tag.
            if html[i..].starts_with("</") {
                let start = i + 2;
                let mut j = start;
                while j < n && bytes[j] != b'>' && !bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let tag = html[start..j].to_ascii_lowercase();
                match html[j..].find('>') {
                    Some(end) => i = j + end + 1,
                    None => break,
                }
                if !tag.is_empty() {
                    tokens.push(Token::EndTag { tag });
                }
                continue;
            }
            // Start tag (must be followed by an ASCII letter).
            if i + 1 < n && bytes[i + 1].is_ascii_alphabetic() {
                let (tag, attrs, self_closing, next) = parse_start_tag(html, i);
                i = next;
                let raw = is_raw_text_tag(&tag) && !self_closing;
                tokens.push(Token::StartTag {
                    tag: tag.clone(),
                    attrs,
                    self_closing,
                });
                if raw {
                    // Consume everything up to the matching end tag as text.
                    let close = format!("</{}", tag);
                    let rest = &html[i..];
                    let lower = rest.to_ascii_lowercase();
                    match lower.find(&close) {
                        Some(end) => {
                            let text = decode_entities(&rest[..end]);
                            if !text.is_empty() {
                                tokens.push(Token::Text(text));
                            }
                            i += end;
                        }
                        None => {
                            let text = decode_entities(rest);
                            if !text.is_empty() {
                                tokens.push(Token::Text(text));
                            }
                            i = n;
                        }
                    }
                }
                continue;
            }
            // A stray '<' is literal text.
            tokens.push(Token::Text("<".to_string()));
            i += 1;
        } else {
            let start = i;
            while i < n && bytes[i] != b'<' {
                i += 1;
            }
            let text = decode_entities(&html[start..i]);
            if !text.is_empty() {
                tokens.push(Token::Text(text));
            }
        }
    }
    tokens
}

/// Parse `<tag attr="v" ...>` starting at `start` (which points at `<`).
/// Returns (tag, attributes, self_closing, index just past `>`).
fn parse_start_tag(html: &str, start: usize) -> (String, HashMap<String, String>, bool, usize) {
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut i = start + 1;

    let tag_start = i;
    while i < n && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b':') {
        i += 1;
    }
    let tag = html[tag_start..i].to_ascii_lowercase();

    let mut attrs = HashMap::new();
    let mut self_closing = false;

    loop {
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        if bytes[i] == b'/' {
            if i + 1 < n && bytes[i + 1] == b'>' {
                self_closing = true;
                i += 2;
                break;
            }
            i += 1;
            continue;
        }
        // Attribute name.
        let name_start = i;
        while i < n
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name = html[name_start..i].to_ascii_lowercase();
        while i < n && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // Optional value.
        let mut value = String::new();
        if i < n && bytes[i] == b'=' {
            i += 1;
            while i < n && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < n && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < n && bytes[i] != quote {
                    i += 1;
                }
                value = decode_entities(&html[v_start..i]);
                if i < n {
                    i += 1; // closing quote
                }
            } else {
                let v_start = i;
                while i < n && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = decode_entities(&html[v_start..i]);
            }
        }
        if !name.is_empty() {
            attrs.insert(name, value);
        }
    }

    (tag, attrs, self_closing, i)
}

/// Decode the most common HTML character references.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let Some(semi) = rest.find(';') else {
            out.push_str(rest);
            break;
        };
        // Only treat it as an entity if ';' is close by.
        if semi > 10 {
            out.push('&');
            rest = &rest[1..];
            continue;
        }
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" => Some("'".to_string()),
            "nbsp" => Some("\u{00A0}".to_string()),
            "copy" => Some("©".to_string()),
            "reg" => Some("®".to_string()),
            "hellip" => Some("…".to_string()),
            "mdash" => Some("—".to_string()),
            "ndash" => Some("–".to_string()),
            "laquo" => Some("«".to_string()),
            "raquo" => Some("»".to_string()),
            _ => {
                if let Some(num) = entity.strip_prefix('#') {
                    let code = if let Some(hex) =
                        num.strip_prefix('x').or_else(|| num.strip_prefix('X'))
                    {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    };
                    code.and_then(char::from_u32).map(|c| c.to_string())
                } else {
                    None
                }
            }
        };
        match decoded {
            Some(d) => {
                out.push_str(&d);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_document() {
        let doc = parse("<html><head><title>Hi</title></head><body><p>Hello</p></body></html>");
        assert_eq!(doc.title, "Hi");
        let body = doc.body().unwrap();
        assert_eq!(body.find_first("p").unwrap().text_content(), "Hello");
    }

    #[test]
    fn handles_attributes_and_entities() {
        let doc = parse(r#"<body><a href="/x?a=1&amp;b=2" class="c1 c2">link</a></body>"#);
        let a = doc.body().unwrap().find_first("a").unwrap();
        assert_eq!(a.attr("href"), Some("/x?a=1&b=2"));
        assert_eq!(a.classes(), vec!["c1", "c2"]);
    }

    #[test]
    fn closes_implied_p_and_li() {
        let doc = parse("<body><p>one<p>two<ul><li>a<li>b</ul></body>");
        let body = doc.body().unwrap();
        assert_eq!(body.find_all("p").len(), 2);
        assert_eq!(body.find_all("li").len(), 2);
    }

    #[test]
    fn style_content_is_raw_text() {
        let doc = parse("<head><style>a > b { color: red }</style></head><body>x</body>");
        assert_eq!(doc.style_texts(), vec!["a > b { color: red }"]);
    }

    #[test]
    fn missing_body_is_synthesised() {
        let doc = parse("<h1>Title</h1><p>text</p>");
        assert!(doc.body().is_some());
        assert_eq!(
            doc.body().unwrap().find_first("h1").unwrap().text_content(),
            "Title"
        );
    }
}

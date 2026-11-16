//! Layout engine: DOM + computed styles -> box tree with absolute rects.
//!
//! Implements a simplified CSS 2.1 block layout:
//! - block containers stack children vertically (with margin collapsing
//!   between siblings, as in real browsers)
//! - inline content (text, links, images) flows into lines with word wrap
//! - `<br>` forces a line break; `<img>` becomes a replaced inline box
//! - every box records its link target (if inside `<a>`) for hit-testing
//!
//! Text measurement uses the shared [`FontSet`](crate::render::FontSet),
//! so wrapping matches rendering exactly.

use crate::css::{style_for, ComputedStyle, Display, InheritedStyle, Stylesheet, TextAlign};
use crate::html::{Document, Element, Node};
use crate::render::FontSet;

#[derive(Debug, Clone, PartialEq)]
pub enum BoxKind {
    Block { tag: String },
    Inline { tag: String },
    Text(String),
    Image { src: String, alt: String },
    Br,
}

/// A laid-out box. Coordinates are absolute page coordinates (before scroll).
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub kind: BoxKind,
    pub style: ComputedStyle,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub children: Vec<LayoutBox>,
    /// Resolved link target if this box is inside an `<a href>`.
    pub link: Option<String>,
}

impl LayoutBox {
    fn new(kind: BoxKind, style: ComputedStyle, link: Option<String>) -> Self {
        Self {
            kind,
            style,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            children: Vec::new(),
            link,
        }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Result of hit-testing: the innermost link under the point, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub link: Option<String>,
}

pub fn hit_test(root: &LayoutBox, x: f32, y: f32) -> Option<Hit> {
    if !root.contains(x, y) {
        return None;
    }
    // Deepest box wins, but links are inherited downwards: a text box inside
    // an `<a>` should report the anchor's href even if it carries none itself.
    for child in root.children.iter().rev() {
        if let Some(mut hit) = hit_test(child, x, y) {
            if hit.link.is_none() {
                hit.link = root.link.clone();
            }
            return Some(hit);
        }
    }
    Some(Hit {
        link: root.link.clone(),
    })
}

/// Lay out a whole document for a viewport of `viewport_width` px.
/// Returns the root block box (the `<html>` element's box).
pub fn layout_document(
    doc: &Document,
    sheet: &Stylesheet,
    viewport_width: f32,
    fonts: &FontSet,
) -> LayoutBox {
    let inherited = InheritedStyle::default();
    let mut ancestors: Vec<&Element> = Vec::new();
    let root_style = style_for(&doc.root, &[], sheet, &inherited);
    let mut root = layout_block(
        &doc.root,
        &root_style,
        sheet,
        &mut ancestors,
        None,
        viewport_width,
        fonts,
    );
    root.x = 0.0;
    root.y = 0.0;
    root.width = viewport_width;
    // Layout builds the tree with child coordinates relative to the parent's
    // border box; flatten them into absolute page coordinates for rendering
    // and hit-testing.
    make_absolute(&mut root, 0.0, 0.0);
    root
}

/// Convert the relative coordinates produced during layout into absolute
/// page coordinates.
fn make_absolute(b: &mut LayoutBox, dx: f32, dy: f32) {
    b.x += dx;
    b.y += dy;
    let (ax, ay) = (b.x, b.y);
    for child in &mut b.children {
        make_absolute(child, ax, ay);
    }
}

/// Lay out one block-level element: resolve its width, then lay out its
/// children inside its content box.
fn layout_block<'a>(
    el: &'a Element,
    style: &ComputedStyle,
    sheet: &Stylesheet,
    ancestors: &mut Vec<&'a Element>,
    link: Option<String>,
    available_width: f32,
    fonts: &FontSet,
) -> LayoutBox {
    let margin = style.margin;
    let padding = style.padding;

    // Content width: explicit width wins, otherwise fill available space.
    let content_width = match style.width {
        Some(len) => len.resolve(available_width),
        None => {
            (available_width - margin.left - margin.right - padding.left - padding.right).max(0.0)
        }
    };
    let box_width = content_width + padding.left + padding.right;

    let mut b = LayoutBox::new(
        BoxKind::Block {
            tag: el.tag.clone(),
        },
        style.clone(),
        link,
    );
    b.width = box_width;

    let content_x = padding.left;
    let mut cursor_y = padding.top;
    let mut prev_margin_bottom = 0.0f32;
    let mut first = true;

    ancestors.push(el);
    let inherited = style.inherited();

    // Group children into block-level and inline runs.
    let mut inline_run: Vec<InlineItem> = Vec::new();

    let flush_inline = |run: &mut Vec<InlineItem>,
                        cursor_y: &mut f32,
                        prev_margin_bottom: &mut f32,
                        first: &mut bool,
                        b: &mut LayoutBox| {
        if run.is_empty() {
            return;
        }
        let (line_boxes, h) = layout_inline_run(
            std::mem::take(run),
            content_x,
            *cursor_y,
            content_width,
            style,
            fonts,
        );
        // Inline content does not collapse margins with siblings the same
        // way blocks do; keep it simple: no extra gap.
        *cursor_y += h;
        *prev_margin_bottom = 0.0;
        *first = false;
        b.children.extend(line_boxes);
    };

    for child in &el.children {
        match child {
            Node::Text(text) => {
                if text.trim().is_empty() && inline_run.is_empty() {
                    continue; // inter-block whitespace
                }
                inline_run.push(InlineItem::Text {
                    text: text.clone(),
                    style: style.clone(),
                    link: b.link.clone(),
                });
            }
            Node::Element(child_el) => {
                let child_style = style_for(child_el, ancestors, sheet, &inherited);
                if child_style.display == Display::None {
                    continue;
                }
                let child_link = match child_el.tag.as_str() {
                    "a" => child_el
                        .attr("href")
                        .map(|h| h.to_string())
                        .or_else(|| b.link.clone()),
                    _ => b.link.clone(),
                };
                match child_style.display {
                    Display::Block => {
                        flush_inline(
                            &mut inline_run,
                            &mut cursor_y,
                            &mut prev_margin_bottom,
                            &mut first,
                            &mut b,
                        );
                        // Margin collapsing between siblings.
                        let gap = if first {
                            child_style.margin.top
                        } else {
                            prev_margin_bottom.max(child_style.margin.top)
                        };
                        cursor_y += gap;
                        let mut child_box = layout_block(
                            child_el,
                            &child_style,
                            sheet,
                            ancestors,
                            child_link,
                            content_width,
                            fonts,
                        );
                        child_box.x = content_x + child_style.margin.left;
                        child_box.y = cursor_y;
                        cursor_y += child_box.height;
                        prev_margin_bottom = child_style.margin.bottom;
                        first = false;
                        b.children.push(child_box);
                    }
                    Display::Inline => {
                        collect_inline(
                            child_el,
                            &child_style,
                            sheet,
                            ancestors,
                            child_link,
                            &mut inline_run,
                        );
                    }
                    Display::None => {}
                }
            }
        }
    }
    flush_inline(
        &mut inline_run,
        &mut cursor_y,
        &mut prev_margin_bottom,
        &mut first,
        &mut b,
    );

    ancestors.pop();

    // Height: explicit height wins, otherwise shrink-wrap content.
    b.height = match style.height {
        Some(len) => len.resolve(available_width) + padding.top + padding.bottom,
        None => cursor_y + padding.bottom,
    };
    b
}

/// Flatten an inline element (and its descendants) into inline items.
fn collect_inline<'a>(
    el: &'a Element,
    style: &ComputedStyle,
    sheet: &Stylesheet,
    ancestors: &mut Vec<&'a Element>,
    link: Option<String>,
    out: &mut Vec<InlineItem>,
) {
    match el.tag.as_str() {
        "br" => {
            out.push(InlineItem::Break);
            return;
        }
        "img" => {
            out.push(InlineItem::Image {
                src: el.attr("src").unwrap_or("").to_string(),
                alt: el.attr("alt").unwrap_or("").to_string(),
                style: style.clone(),
                link: link.clone(),
            });
            return;
        }
        "hr" => {
            // Treat as a block-ish break with a gap.
            out.push(InlineItem::Break);
            return;
        }
        _ => {}
    }

    ancestors.push(el);
    let inherited = style.inherited();
    for child in &el.children {
        match child {
            Node::Text(text) => {
                out.push(InlineItem::Text {
                    text: text.clone(),
                    style: style.clone(),
                    link: link.clone(),
                });
            }
            Node::Element(child_el) => {
                let child_style = style_for(child_el, ancestors, sheet, &inherited);
                if child_style.display == Display::None {
                    continue;
                }
                let child_link = match child_el.tag.as_str() {
                    "a" => child_el
                        .attr("href")
                        .map(|h| h.to_string())
                        .or_else(|| link.clone()),
                    _ => link.clone(),
                };
                if child_style.display == Display::Block {
                    // Block inside inline: degrade gracefully — break line,
                    // then collect its inline content.
                    out.push(InlineItem::Break);
                    collect_inline(child_el, &child_style, sheet, ancestors, child_link, out);
                    out.push(InlineItem::Break);
                } else {
                    collect_inline(child_el, &child_style, sheet, ancestors, child_link, out);
                }
            }
        }
    }
    ancestors.pop();
}

enum InlineItem {
    Text {
        text: String,
        style: ComputedStyle,
        link: Option<String>,
    },
    Image {
        src: String,
        alt: String,
        style: ComputedStyle,
        link: Option<String>,
    },
    Break,
}

/// A word (or image) ready to be placed on a line.
struct Word {
    text: Option<String>,
    image: Option<(String, String)>,
    style: ComputedStyle,
    link: Option<String>,
    width: f32,
    height: f32,
    space_before: bool,
}

/// Lay out a run of inline items into line boxes (word wrap).
fn layout_inline_run(
    items: Vec<InlineItem>,
    x: f32,
    y: f32,
    width: f32,
    container_style: &ComputedStyle,
    fonts: &FontSet,
) -> (Vec<LayoutBox>, f32) {
    // 1. Split items into words.
    let mut words: Vec<Word> = Vec::new();
    // Trailing whitespace in one inline item separates it from the first word
    // of the next item (`text <b>bold</b>` / `text <a>link</a>`).
    let mut pending_space = false;
    for item in items {
        match item {
            InlineItem::Break => {
                pending_space = false;
                words.push(Word {
                    text: None,
                    image: None,
                    style: container_style.clone(),
                    link: None,
                    width: 0.0,
                    height: 0.0,
                    space_before: false,
                });
            }
            InlineItem::Image {
                src,
                alt,
                style,
                link,
            } => {
                let space_before = pending_space && !words.is_empty();
                pending_space = false;
                let w = style.width.map(|l| l.resolve(width)).unwrap_or(64.0);
                let h = style.height.map(|l| l.resolve(width)).unwrap_or(64.0);
                words.push(Word {
                    text: None,
                    image: Some((src, alt)),
                    style,
                    link,
                    width: w,
                    height: h,
                    space_before,
                });
            }
            InlineItem::Text { text, style, link } => {
                let size = style.font_size;
                let bold = style.font_weight >= 600;
                let line_h = fonts.line_height(size) * style.line_height;
                // Split on whitespace. A run of whitespace *after* a word
                // becomes a gap before the next word (`space_before`).
                // Whitespace left over from the previous item becomes the
                // gap before this item's first word.
                // Whitespace can arrive either as the tail of the previous
                // inline item or as the head of this one (`a <b>x</b> y`).
                let gap_before_first =
                    (pending_space || text.starts_with(char::is_whitespace)) && !words.is_empty();
                pending_space = false;
                let mut first_word_of_item = true;
                let mut word_start: Option<usize> = None;
                for (idx, c) in text.char_indices() {
                    if c.is_whitespace() {
                        if let Some(start) = word_start.take() {
                            let trimmed = &text[start..idx];
                            let w = fonts.width(trimmed, size, bold, style.italic);
                            words.push(Word {
                                text: Some(trimmed.to_string()),
                                image: None,
                                style: style.clone(),
                                link: link.clone(),
                                width: w,
                                height: line_h,
                                space_before: if first_word_of_item {
                                    gap_before_first
                                } else {
                                    pending_space
                                },
                            });
                            first_word_of_item = false;
                        }
                        pending_space = true;
                    } else if word_start.is_none() {
                        word_start = Some(idx);
                    }
                }
                if let Some(start) = word_start {
                    let trimmed = &text[start..];
                    let w = fonts.width(trimmed, size, bold, style.italic);
                    words.push(Word {
                        text: Some(trimmed.to_string()),
                        image: None,
                        style: style.clone(),
                        link: link.clone(),
                        width: w,
                        height: line_h,
                        space_before: if first_word_of_item {
                            gap_before_first
                        } else {
                            pending_space
                        },
                    });
                }
                // Trailing whitespace separates this item from the next one.
                if text.ends_with(char::is_whitespace) {
                    pending_space = true;
                }
            }
        }
    }

    // 2. Greedy line breaking.
    let mut lines: Vec<Vec<Word>> = Vec::new();
    let mut current: Vec<Word> = Vec::new();
    let mut current_w = 0.0f32;
    for word in words {
        if word.text.is_none() && word.image.is_none() {
            // Explicit break.
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
            continue;
        }
        let space_w = if word.space_before && !current.is_empty() {
            fonts.width(
                " ",
                word.style.font_size,
                word.style.font_weight >= 600,
                word.style.italic,
            )
        } else {
            0.0
        };
        if !current.is_empty() && current_w + space_w + word.width > width {
            lines.push(std::mem::take(&mut current));
            current_w = 0.0;
        }
        current_w += (if current.is_empty() { 0.0 } else { space_w }) + word.width;
        current.push(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }

    // 3. Build boxes per line.
    let mut boxes = Vec::new();
    let mut cy = y;
    let start_y = y;
    for line in lines {
        let line_h = line.iter().map(|w| w.height).fold(0.0f32, f32::max);
        let total_w: f32 = line
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let sp = if i > 0 && w.space_before {
                    fonts.width(
                        " ",
                        w.style.font_size,
                        w.style.font_weight >= 600,
                        w.style.italic,
                    )
                } else {
                    0.0
                };
                sp + w.width
            })
            .sum();
        let mut cx = match container_style.text_align {
            TextAlign::Center => x + ((width - total_w) / 2.0).max(0.0),
            TextAlign::Right => x + (width - total_w).max(0.0),
            TextAlign::Left => x,
        };
        for (i, word) in line.into_iter().enumerate() {
            if i > 0 && word.space_before {
                cx += fonts.width(
                    " ",
                    word.style.font_size,
                    word.style.font_weight >= 600,
                    word.style.italic,
                );
            }
            let mut b = match (&word.text, &word.image) {
                (Some(text), _) => {
                    let mut bx = LayoutBox::new(
                        BoxKind::Text(text.clone()),
                        word.style.clone(),
                        word.link.clone(),
                    );
                    bx.width = word.width;
                    bx.height = word.height;
                    bx
                }
                (None, Some((src, alt))) => {
                    let mut bx = LayoutBox::new(
                        BoxKind::Image {
                            src: src.clone(),
                            alt: alt.clone(),
                        },
                        word.style.clone(),
                        word.link.clone(),
                    );
                    bx.width = word.width;
                    bx.height = word.height;
                    bx
                }
                _ => continue,
            };
            b.x = cx;
            b.y = cy;
            cx += word.width;
            boxes.push(b);
        }
        cy += line_h;
    }
    (boxes, cy - start_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::Stylesheet;
    use crate::html;

    fn layout(html: &str, width: f32) -> LayoutBox {
        let doc = html::parse(html);
        let sheet = crate::css::ua_stylesheet();
        let fonts = FontSet::load();
        layout_document(&doc, &sheet, width, &fonts)
    }

    #[test]
    fn lays_out_paragraph() {
        let root = layout("<body><p>Hello world</p></body>", 800.0);
        assert!(root.height > 0.0);
        assert!(!root.children.is_empty());
    }

    #[test]
    fn wraps_long_text() {
        let long = "word ".repeat(100);
        let root = layout(&format!("<body><p>{}</p></body>", long), 200.0);
        // Wrapped text must produce multiple line boxes.
        fn count_text(b: &LayoutBox) -> usize {
            let mut n = if matches!(b.kind, BoxKind::Text(_)) {
                1
            } else {
                0
            };
            for c in &b.children {
                n += count_text(c);
            }
            n
        }
        assert!(count_text(&root) > 10);
    }

    #[test]
    fn block_height_matches_line_count() {
        // Two words on one line must not inflate the block height.
        let root = layout("<body><p>Hello world</p><p>second</p></body>", 800.0);
        let body = &root.children[0];
        let p1 = &body.children[0];
        let p2 = &body.children[1];
        let line_h = p1.children[0].height;
        assert!(
            (p1.height - line_h).abs() < 0.5,
            "p1 height {} vs line {}",
            p1.height,
            line_h
        );
        assert!(
            (p2.height - line_h).abs() < 0.5,
            "p2 height {} vs line {}",
            p2.height,
            line_h
        );
    }

    #[test]
    fn words_are_separated_by_spaces() {
        // Consecutive words must advance past a space, not sit flush.
        let root = layout("<body><p>aaa bbb</p></body>", 800.0);
        let p = &root.children[0].children[0];
        let (a, b) = (&p.children[0], &p.children[1]);
        assert!(
            b.x >= a.x + a.width,
            "words overlap: {} vs {}",
            b.x,
            a.x + a.width
        );
        assert!(b.x - (a.x + a.width) > 1.0, "no space between words");
    }

    #[test]
    fn blocks_stack_without_overlap() {
        let root = layout("<body><p>one</p><p>two</p><p>three</p></body>", 800.0);
        let body = &root.children[0];
        let mut bottom = 0.0f32;
        for p in &body.children {
            assert!(
                p.y >= bottom,
                "block at y={} overlaps previous bottom={}",
                p.y,
                bottom
            );
            bottom = p.y + p.height;
        }
        assert!(
            bottom <= root.height + 0.5,
            "content {} exceeds root {}",
            bottom,
            root.height
        );
    }

    #[test]
    fn hit_test_finds_links() {
        let root = layout(
            r#"<body><p><a href="https://example.com">click</a></p></body>"#,
            800.0,
        );
        // Find the link box and hit-test its centre.
        fn find_link(b: &LayoutBox) -> Option<(f32, f32)> {
            if b.link.is_some() {
                return Some((b.x + b.width / 2.0, b.y + b.height / 2.0));
            }
            for c in &b.children {
                if let Some(p) = find_link(c) {
                    return Some(p);
                }
            }
            None
        }
        let (x, y) = find_link(&root).expect("link box");
        let hit = hit_test(&root, x, y).expect("hit");
        assert_eq!(hit.link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn display_none_hides_content() {
        let sheet = Stylesheet::parse(".hidden { display: none }");
        let doc = html::parse(r#"<body><p class="hidden">secret</p><p>shown</p></body>"#);
        let fonts = FontSet::load();
        let root = layout_document(&doc, &sheet, 800.0, &fonts);
        fn all_text(b: &LayoutBox, out: &mut String) {
            if let BoxKind::Text(t) = &b.kind {
                out.push_str(t);
            }
            for c in &b.children {
                all_text(c, out);
            }
        }
        let mut text = String::new();
        all_text(&root, &mut text);
        assert!(text.contains("shown"));
        assert!(!text.contains("secret"));
    }
}

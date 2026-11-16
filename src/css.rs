//! CSS parser and style resolution, written from scratch.
//!
//! Supported subset (v0.1):
//! - selectors: `*`, `tag`, `.class`, `#id`, compounds (`div.card`),
//!   descendant combinators (`div p`), comma groups
//! - declarations: `color`, `background` / `background-color`,
//!   `font-size`, `font-weight`, `font-style`, `font-family` (parsed, kept),
//!   `margin` / `padding` (+ longhands), `display`, `text-align`,
//!   `line-height`, `width`, `height`, `text-decoration`
//! - colours: named colours, `#rgb`, `#rrggbb`, `rgb(r,g,b)`
//! - lengths: `px`, `pt`, `em`, `%` (percentages resolve against the
//!   containing block during layout; here we keep the raw value)
//!
//! Cascade: UA stylesheet < author stylesheet; within one origin,
//! higher specificity wins, ties broken by source order. Inherited
//! properties (`color`, `font-size`, ...) flow down the tree.

use crate::html::Element;

/// An sRGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if let Some(hex) = s.strip_prefix('#') {
            return match hex.len() {
                3 => {
                    let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                    let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                    let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                    Some(Self::rgb(r, g, b))
                }
                6 => {
                    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                    Some(Self::rgb(r, g, b))
                }
                _ => None,
            };
        }
        if s.starts_with("rgb(") && s.ends_with(')') {
            let inner = &s[4..s.len() - 1];
            let parts: Vec<&str> = inner.split(',').collect();
            if parts.len() == 3 {
                let r = parts[0].trim().parse::<u8>().ok()?;
                let g = parts[1].trim().parse::<u8>().ok()?;
                let b = parts[2].trim().parse::<u8>().ok()?;
                return Some(Self::rgb(r, g, b));
            }
            return None;
        }
        named_color(&s)
    }
}

fn named_color(name: &str) -> Option<Color> {
    Some(match name {
        "black" => Color::rgb(0, 0, 0),
        "white" => Color::rgb(255, 255, 255),
        "red" => Color::rgb(255, 0, 0),
        "green" => Color::rgb(0, 128, 0),
        "lime" => Color::rgb(0, 255, 0),
        "blue" => Color::rgb(0, 0, 255),
        "yellow" => Color::rgb(255, 255, 0),
        "cyan" | "aqua" => Color::rgb(0, 255, 255),
        "magenta" | "fuchsia" => Color::rgb(255, 0, 255),
        "orange" => Color::rgb(255, 165, 0),
        "purple" => Color::rgb(128, 0, 128),
        "pink" => Color::rgb(255, 192, 203),
        "brown" => Color::rgb(165, 42, 42),
        "gray" | "grey" => Color::rgb(128, 128, 128),
        "silver" => Color::rgb(192, 192, 192),
        "maroon" => Color::rgb(128, 0, 0),
        "olive" => Color::rgb(128, 128, 0),
        "navy" => Color::rgb(0, 0, 128),
        "teal" => Color::rgb(0, 128, 128),
        "skyblue" => Color::rgb(135, 206, 235),
        "lightblue" => Color::rgb(173, 216, 230),
        "lightgray" | "lightgrey" => Color::rgb(211, 211, 211),
        "darkgray" | "darkgrey" => Color::rgb(169, 169, 169),
        "darkblue" => Color::rgb(0, 0, 139),
        "transparent" => Color::rgb(0, 0, 0),
        _ => return None,
    })
}

/// A CSS length: absolute pixels or a percentage of the containing block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    Px(f32),
    Percent(f32),
}

impl Length {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if let Some(pct) = s.strip_suffix('%') {
            return pct.trim().parse::<f32>().ok().map(Length::Percent);
        }
        let num = s
            .strip_suffix("px")
            .or_else(|| s.strip_suffix("pt"))
            .unwrap_or(&s);
        let v: f32 = num.trim().parse().ok()?;
        // 1pt = 1.333px, but for a toy engine we keep pt == px * 1.33.
        let v = if s.ends_with("pt") { v * 1.333 } else { v };
        Some(Length::Px(v))
    }

    /// Resolve against a containing-block size (for percentages).
    pub fn resolve(self, basis: f32) -> f32 {
        match self {
            Length::Px(v) => v,
            Length::Percent(p) => basis * p / 100.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Box sides in CSS order: top, right, bottom, left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sides {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Properties that inherit from parent to child.
#[derive(Debug, Clone, Copy)]
pub struct InheritedStyle {
    pub color: Color,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub text_align: TextAlign,
    pub line_height: f32,
}

impl Default for InheritedStyle {
    fn default() -> Self {
        Self {
            color: Color::rgb(0, 0, 0),
            font_size: 16.0,
            font_weight: 400,
            italic: false,
            text_align: TextAlign::Left,
            line_height: 1.4,
        }
    }
}

/// Fully resolved style for one element.
#[derive(Debug, Clone)]
pub struct ComputedStyle {
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub text_align: TextAlign,
    pub line_height: f32,
    pub display: Display,
    pub margin: Sides,
    pub padding: Sides,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub underline: bool,
}

impl ComputedStyle {
    pub fn from_inherited(inherited: &InheritedStyle, display: Display) -> Self {
        Self {
            color: inherited.color,
            background: None,
            font_size: inherited.font_size,
            font_weight: inherited.font_weight,
            italic: inherited.italic,
            text_align: inherited.text_align,
            line_height: inherited.line_height,
            display,
            margin: Sides::default(),
            padding: Sides::default(),
            width: None,
            height: None,
            underline: false,
        }
    }

    pub fn inherited(&self) -> InheritedStyle {
        InheritedStyle {
            color: self.color,
            font_size: self.font_size,
            font_weight: self.font_weight,
            italic: self.italic,
            text_align: self.text_align,
            line_height: self.line_height,
        }
    }
}

/// One compound selector, e.g. `div.card#main` or `*`.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleSelector {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub universal: bool,
}

/// A full selector: compounds joined by descendant combinators.
/// The last compound matches the element itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub parts: Vec<SimpleSelector>,
}

impl Selector {
    /// CSS specificity as (ids, classes, tags) — compared lexicographically.
    pub fn specificity(&self) -> (usize, usize, usize) {
        let mut ids = 0;
        let mut classes = 0;
        let mut tags = 0;
        for p in &self.parts {
            if p.id.is_some() {
                ids += 1;
            }
            classes += p.classes.len();
            if p.tag.is_some() {
                tags += 1;
            }
        }
        (ids, classes, tags)
    }

    pub fn matches(&self, element: &Element, ancestors: &[&Element]) -> bool {
        if self.parts.is_empty() {
            return false;
        }
        let (last, rest) = self.parts.split_last().unwrap();
        if !matches_simple(last, element) {
            return false;
        }
        // Descendant combinators: walk ancestors right-to-left.
        let mut ai = ancestors.len();
        for part in rest.iter().rev() {
            let mut found = false;
            while ai > 0 {
                ai -= 1;
                if matches_simple(part, ancestors[ai]) {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
    }
}

fn matches_simple(sel: &SimpleSelector, el: &Element) -> bool {
    if sel.universal && sel.tag.is_none() && sel.id.is_none() && sel.classes.is_empty() {
        return true;
    }
    if let Some(tag) = &sel.tag {
        if &el.tag != tag {
            return false;
        }
    }
    if let Some(id) = &sel.id {
        if el.attr("id") != Some(id.as_str()) {
            return false;
        }
    }
    if !sel.classes.is_empty() {
        let el_classes = el.classes();
        for c in &sel.classes {
            if !el_classes.iter().any(|e| e == c) {
                return false;
            }
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
}

/// A parsed stylesheet: an ordered list of rules.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

impl Stylesheet {
    pub fn parse(css: &str) -> Self {
        let css = strip_comments(css);
        let mut rules = Vec::new();
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let selector_text = rest[..open].trim();
            let after_open = &rest[open + 1..];
            let Some(close) = find_close_brace(after_open) else {
                break;
            };
            let body = &after_open[..close];
            rest = &after_open[close + 1..];
            if selector_text.is_empty() || selector_text.starts_with('@') {
                continue; // skip @-rules (media queries etc.) for now
            }
            let selectors: Vec<Selector> = selector_text
                .split(',')
                .filter_map(parse_selector)
                .collect();
            if selectors.is_empty() {
                continue;
            }
            let declarations = parse_declarations(body);
            if !declarations.is_empty() {
                rules.push(Rule {
                    selectors,
                    declarations,
                });
            }
        }
        Self { rules }
    }

    /// Merge another stylesheet after this one (later rules win ties).
    pub fn extend(&mut self, other: Stylesheet) {
        self.rules.extend(other.rules);
    }
}

fn find_close_brace(s: &str) -> Option<usize> {
    // No nested blocks in our subset; first '}' closes the rule.
    s.find('}')
}

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => break,
        }
    }
    out.push_str(rest);
    out
}

fn parse_selector(text: &str) -> Option<Selector> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for compound in text.split_whitespace() {
        parts.push(parse_simple_selector(compound)?);
    }
    if parts.is_empty() {
        None
    } else {
        Some(Selector { parts })
    }
}

fn parse_simple_selector(text: &str) -> Option<SimpleSelector> {
    let mut sel = SimpleSelector {
        tag: None,
        id: None,
        classes: Vec::new(),
        universal: false,
    };
    let mut rest = text;
    // Optional leading tag or '*'.
    if let Some(stripped) = rest.strip_prefix('*') {
        sel.universal = true;
        rest = stripped;
    } else {
        let tag_end = rest.find(['.', '#']).unwrap_or(rest.len());
        if tag_end > 0 {
            sel.tag = Some(rest[..tag_end].to_ascii_lowercase());
            rest = &rest[tag_end..];
        }
    }
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '#']).unwrap_or(r.len());
            if end == 0 {
                return None;
            }
            sel.classes.push(r[..end].to_string());
            rest = &r[end..];
        } else {
            let r = rest.strip_prefix('#')?;
            let end = r.find(['.', '#']).unwrap_or(r.len());
            if end == 0 {
                return None;
            }
            sel.id = Some(r[..end].to_string());
            rest = &r[end..];
        }
    }
    Some(sel)
}

fn parse_declarations(body: &str) -> Vec<Declaration> {
    body.split(';')
        .filter_map(|part| {
            let (prop, value) = part.split_once(':')?;
            let prop = prop.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if prop.is_empty() || value.is_empty() {
                None
            } else {
                Some(Declaration {
                    property: prop,
                    value,
                })
            }
        })
        .collect()
}

/// Default display for a tag when no stylesheet says otherwise.
pub fn default_display(tag: &str) -> Display {
    match tag {
        "head" | "title" | "style" | "script" | "meta" | "link" => Display::None,
        "a" | "span" | "b" | "strong" | "i" | "em" | "u" | "code" | "small" | "sub" | "sup"
        | "img" | "br" | "button" | "input" | "label" => Display::Inline,
        _ => Display::Block,
    }
}

/// The built-in user-agent stylesheet: browser defaults.
pub fn ua_stylesheet() -> Stylesheet {
    Stylesheet::parse(
        r#"
        body { margin: 8px; font-size: 16px; color: #000000; line-height: 1.4; }
        h1 { font-size: 32px; font-weight: 700; margin: 21px 0; }
        h2 { font-size: 24px; font-weight: 700; margin: 19px 0; }
        h3 { font-size: 19px; font-weight: 700; margin: 19px 0; }
        h4 { font-size: 16px; font-weight: 700; margin: 21px 0; }
        h5 { font-size: 13px; font-weight: 700; margin: 22px 0; }
        h6 { font-size: 11px; font-weight: 700; margin: 25px 0; }
        p { margin: 16px 0; }
        ul, ol { margin: 16px 0; padding-left: 40px; }
        li { margin: 0; }
        a { color: #0000ee; text-decoration: underline; }
        b, strong { font-weight: 700; }
        i, em { font-style: italic; }
        code { font-size: 14px; }
        hr { margin: 8px 0; }
        pre { margin: 16px 0; }
        "#,
    )
}

/// Resolve the computed style for `element` given its ancestors
/// (root-first, excluding the element itself), the author stylesheet
/// and the inherited values from its parent.
pub fn style_for(
    element: &Element,
    ancestors: &[&Element],
    sheet: &Stylesheet,
    inherited: &InheritedStyle,
) -> ComputedStyle {
    let mut style = ComputedStyle::from_inherited(inherited, default_display(&element.tag));

    // Presentational hints from HTML attributes (lowest author priority).
    if element.tag == "a" {
        style.underline = true;
    }
    if element.tag == "b" || element.tag == "strong" {
        style.font_weight = 700;
    }
    if element.tag == "i" || element.tag == "em" {
        style.italic = true;
    }

    // Collect matching declarations with (specificity, order).
    let mut matched: Vec<((usize, usize, usize), usize, &Declaration)> = Vec::new();
    for (order, rule) in sheet.rules.iter().enumerate() {
        for sel in &rule.selectors {
            if sel.matches(element, ancestors) {
                let spec = sel.specificity();
                for decl in &rule.declarations {
                    matched.push((spec, order, decl));
                }
            }
        }
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    for (_, _, decl) in matched {
        apply_declaration(&mut style, &decl.property, &decl.value, inherited.font_size);
    }
    style
}

fn apply_declaration(style: &mut ComputedStyle, prop: &str, value: &str, parent_font_size: f32) {
    match prop {
        "color" => {
            if let Some(c) = Color::parse(value) {
                style.color = c;
            }
        }
        "background" | "background-color" => {
            // `background` shorthand: take the first colour token.
            let token = value.split_whitespace().next().unwrap_or("");
            if let Some(c) = Color::parse(token) {
                style.background = Some(c);
            }
        }
        "font-size" => {
            if let Some(len) = Length::parse(value) {
                style.font_size = match len {
                    Length::Px(v) => v,
                    Length::Percent(p) => parent_font_size * p / 100.0,
                };
            } else if value.eq_ignore_ascii_case("small") {
                style.font_size = 13.0;
            } else if value.eq_ignore_ascii_case("large") {
                style.font_size = 20.0;
            } else if value.eq_ignore_ascii_case("medium") {
                style.font_size = 16.0;
            }
        }
        "font-weight" => {
            let v = value.trim().to_ascii_lowercase();
            style.font_weight = match v.as_str() {
                "bold" | "bolder" => 700,
                "normal" => 400,
                "lighter" => 300,
                _ => v.parse::<u16>().unwrap_or(style.font_weight),
            };
        }
        "font-style" => {
            style.italic = value.trim().eq_ignore_ascii_case("italic")
                || value.trim().eq_ignore_ascii_case("oblique");
        }
        "display" => {
            style.display = match value.trim().to_ascii_lowercase().as_str() {
                "none" => Display::None,
                "inline" => Display::Inline,
                _ => Display::Block,
            };
        }
        "text-align" => {
            style.text_align = match value.trim().to_ascii_lowercase().as_str() {
                "center" => TextAlign::Center,
                "right" => TextAlign::Right,
                _ => TextAlign::Left,
            };
        }
        "line-height" => {
            if let Ok(v) = value.trim().parse::<f32>() {
                style.line_height = v;
            } else if let Some(Length::Px(v)) = Length::parse(value) {
                style.line_height = v / style.font_size.max(1.0);
            }
        }
        "text-decoration" => {
            style.underline = value.trim().to_ascii_lowercase().contains("underline");
        }
        "margin" => style.margin = parse_sides(value),
        "margin-top" => {
            if let Some(v) = Length::parse(value) {
                style.margin.top = v.resolve(0.0);
            }
        }
        "margin-right" => {
            if let Some(v) = Length::parse(value) {
                style.margin.right = v.resolve(0.0);
            }
        }
        "margin-bottom" => {
            if let Some(v) = Length::parse(value) {
                style.margin.bottom = v.resolve(0.0);
            }
        }
        "margin-left" => {
            if let Some(v) = Length::parse(value) {
                style.margin.left = v.resolve(0.0);
            }
        }
        "padding" => style.padding = parse_sides(value),
        "padding-top" => {
            if let Some(v) = Length::parse(value) {
                style.padding.top = v.resolve(0.0);
            }
        }
        "padding-right" => {
            if let Some(v) = Length::parse(value) {
                style.padding.right = v.resolve(0.0);
            }
        }
        "padding-bottom" => {
            if let Some(v) = Length::parse(value) {
                style.padding.bottom = v.resolve(0.0);
            }
        }
        "padding-left" => {
            if let Some(v) = Length::parse(value) {
                style.padding.left = v.resolve(0.0);
            }
        }
        "width" => style.width = Length::parse(value),
        "height" => style.height = Length::parse(value),
        _ => {} // unknown properties are ignored (forward-compatible)
    }
}

fn parse_sides(value: &str) -> Sides {
    let vals: Vec<f32> = value
        .split_whitespace()
        .filter_map(|v| Length::parse(v).map(|l| l.resolve(0.0)))
        .collect();
    match vals.len() {
        1 => Sides {
            top: vals[0],
            right: vals[0],
            bottom: vals[0],
            left: vals[0],
        },
        2 => Sides {
            top: vals[0],
            right: vals[1],
            bottom: vals[0],
            left: vals[1],
        },
        3 => Sides {
            top: vals[0],
            right: vals[1],
            bottom: vals[2],
            left: vals[1],
        },
        _ => Sides {
            top: vals.first().copied().unwrap_or(0.0),
            right: vals.get(1).copied().unwrap_or(0.0),
            bottom: vals.get(2).copied().unwrap_or(0.0),
            left: vals.get(3).copied().unwrap_or(0.0),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rules_and_selectors() {
        let sheet = Stylesheet::parse(
            "div.card, #main { color: red; margin: 4px 8px; } p { font-size: 20px }",
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selectors.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
    }

    #[test]
    fn specificity_orders_id_over_class_over_tag() {
        let sheet = Stylesheet::parse("p { color: red } .c { color: green } #i { color: blue }");
        let mut el = Element::new("p");
        el.attributes.insert("class".into(), "c".into());
        el.attributes.insert("id".into(), "i".into());
        let style = style_for(&el, &[], &sheet, &InheritedStyle::default());
        assert_eq!(style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn descendant_selector_matches() {
        let sheet = Stylesheet::parse("div p { color: red }");
        let div = Element::new("div");
        let p = Element::new("p");
        let style = style_for(&p, &[&div], &sheet, &InheritedStyle::default());
        assert_eq!(style.color, Color::rgb(255, 0, 0));
        let style2 = style_for(&p, &[], &sheet, &InheritedStyle::default());
        assert_eq!(style2.color, Color::rgb(0, 0, 0));
    }

    #[test]
    fn parses_colours() {
        assert_eq!(Color::parse("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(Color::parse("#00ff00"), Some(Color::rgb(0, 255, 0)));
        assert_eq!(Color::parse("rgb(1, 2, 3)"), Some(Color::rgb(1, 2, 3)));
        assert_eq!(Color::parse("skyblue"), Some(Color::rgb(135, 206, 235)));
    }
}

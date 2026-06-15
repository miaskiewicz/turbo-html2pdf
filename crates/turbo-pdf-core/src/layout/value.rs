//! Typed CSS value resolution (§5, §4.1). The cascade ([`ComputedStyle`]) keeps
//! values as raw strings; layout needs typed numbers. This module parses lengths,
//! colors, and keywords into the [`BoxStyle`] a box uses for layout.
//!
//! **Internal unit: CSS pixels at 96dpi.** Absolute units convert on parse
//! (`1pt = 1/72in`, `1in = 96px`); `em` resolves against the parent font size and
//! `%` against the containing-block width at resolution time.

use crate::style::ComputedStyle;

/// Pixels per inch for absolute-unit conversion (the CSS reference pixel).
const DPI: f32 = 96.0;
/// The initial font size when none is inherited or set (`16px`).
pub const DEFAULT_FONT_SIZE: f32 = 16.0;

/// Four edge values (margin, padding, border widths), in px.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    /// A uniform edge set.
    pub fn all(v: f32) -> Edges {
        Edges {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    /// Total horizontal extent (left + right).
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// Total vertical extent (top + bottom).
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// A `<length-percentage>` or `auto`, resolved against context as needed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPct {
    Auto,
    Px(f32),
    Pct(f32),
}

impl LengthPct {
    /// Resolve to px against a percentage basis, or `None` for `auto`.
    pub fn resolve(&self, basis: f32) -> Option<f32> {
        match self {
            LengthPct::Auto => None,
            LengthPct::Px(v) => Some(*v),
            LengthPct::Pct(p) => Some(p / 100.0 * basis),
        }
    }
}

/// CSS `display` (the v1 subset, §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    Flex,
    None,
    Table,
    TableRow,
    TableCell,
    TableHeaderGroup,
    TableFooterGroup,
    ListItem,
}

/// CSS `box-sizing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// CSS `vertical-align` (the inline/table-cell subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Baseline,
    Sub,
    Super,
    Middle,
    Top,
    Bottom,
}

/// CSS fragmentation rule (`break-before`/`break-after`/`break-inside`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakRule {
    #[default]
    Auto,
    Avoid,
    Page,
}

/// An RGBA color, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba {
        Rgba { r, g, b, a }
    }
}

/// One border side: width (px) and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderSide {
    pub width: u16,
    pub color: Option<Rgba>,
}

/// The four border sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BorderEdges {
    pub top: BorderSide,
    pub right: BorderSide,
    pub bottom: BorderSide,
    pub left: BorderSide,
}

impl BorderEdges {
    /// Whether any side has a non-zero width, i.e. the border paints something
    /// (used by the `pdf-ua` emitter to decide if a box is decoration). Gated so
    /// it adds nothing to the default build's coverage surface (AC-11.1).
    #[cfg(feature = "pdf-ua")]
    pub fn any_visible(&self) -> bool {
        self.top.width > 0 || self.right.width > 0 || self.bottom.width > 0 || self.left.width > 0
    }

    /// Border widths as plain px [`Edges`].
    pub fn widths(&self) -> Edges {
        Edges {
            top: f32::from(self.top.width),
            right: f32::from(self.right.width),
            bottom: f32::from(self.bottom.width),
            left: f32::from(self.left.width),
        }
    }
}

// --------------------------------------------------------------------------
// length parsing
// --------------------------------------------------------------------------

/// A parsed length before `em`/`%` resolution.
#[derive(Debug, Clone, Copy, PartialEq)]
enum RawLength {
    Abs(f32),
    Em(f32),
    Pct(f32),
}

fn unit_factor(unit: &str) -> Option<f32> {
    match unit {
        "px" | "" => Some(1.0),
        "pt" => Some(DPI / 72.0),
        "pc" => Some(DPI / 6.0),
        "in" => Some(DPI),
        "cm" => Some(DPI / 2.54),
        "mm" => Some(DPI / 25.4),
        _ => None,
    }
}

fn split_unit(s: &str) -> (&str, &str) {
    let end = s
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic() || *c == '%')
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

fn parse_raw(s: &str) -> Option<RawLength> {
    let t = s.trim();
    let (num, unit) = split_unit(t);
    let value: f32 = num.parse().ok()?;
    match unit {
        "%" => Some(RawLength::Pct(value)),
        "em" | "rem" => Some(RawLength::Em(value)),
        u => unit_factor(u).map(|f| RawLength::Abs(value * f)),
    }
}

fn raw_to_px(raw: RawLength, font_size: f32, basis: f32) -> f32 {
    match raw {
        RawLength::Abs(v) => v,
        RawLength::Em(v) => v * font_size,
        RawLength::Pct(p) => p / 100.0 * basis,
    }
}

/// Parse an absolute/`em` length to px (no `%`); used for font-relative values.
pub fn parse_px(s: &str, font_size: f32) -> Option<f32> {
    match parse_raw(s)? {
        RawLength::Pct(_) => None,
        raw => Some(raw_to_px(raw, font_size, 0.0)),
    }
}

/// Parse a `<length-percentage>`/`auto` into a [`LengthPct`].
pub fn parse_length_pct(s: &str, font_size: f32) -> Option<LengthPct> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("auto") {
        return Some(LengthPct::Auto);
    }
    match parse_raw(t)? {
        RawLength::Pct(p) => Some(LengthPct::Pct(p)),
        RawLength::Abs(v) => Some(LengthPct::Px(v)),
        RawLength::Em(v) => Some(LengthPct::Px(v * font_size)),
    }
}

// --------------------------------------------------------------------------
// color parsing
// --------------------------------------------------------------------------

fn named_color(name: &str) -> Option<Rgba> {
    let c = match name {
        "black" => (0, 0, 0, 255),
        "white" => (255, 255, 255, 255),
        "red" => (255, 0, 0, 255),
        "green" => (0, 128, 0, 255),
        "blue" => (0, 0, 255, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "transparent" => (0, 0, 0, 0),
        _ => return None,
    };
    Some(Rgba::new(c.0, c.1, c.2, c.3))
}

fn hex_pair(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

fn parse_hex3(h: &str) -> Option<Rgba> {
    let dup = |c: &str| hex_pair(&format!("{c}{c}"));
    Some(Rgba::new(
        dup(&h[0..1])?,
        dup(&h[1..2])?,
        dup(&h[2..3])?,
        255,
    ))
}

fn parse_hex6(h: &str) -> Option<Rgba> {
    Some(Rgba::new(
        hex_pair(&h[0..2])?,
        hex_pair(&h[2..4])?,
        hex_pair(&h[4..6])?,
        255,
    ))
}

fn parse_hex8(h: &str) -> Option<Rgba> {
    let mut c = parse_hex6(&h[0..6])?;
    c.a = hex_pair(&h[6..8])?;
    Some(c)
}

fn parse_hex(h: &str) -> Option<Rgba> {
    match h.len() {
        3 => parse_hex3(h),
        6 => parse_hex6(h),
        8 => parse_hex8(h),
        _ => None,
    }
}

fn channel(part: &str) -> Option<u8> {
    let v: f32 = part.trim().parse().ok()?;
    Some(v.round().clamp(0.0, 255.0) as u8)
}

fn alpha_channel(part: &str) -> Option<u8> {
    let v: f32 = part.trim().parse().ok()?;
    Some((v * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn nth_channel(parts: &[&str], i: usize) -> Option<u8> {
    channel(parts.get(i)?)
}

fn parse_rgb_parts(parts: &[&str]) -> Option<Rgba> {
    Some(Rgba::new(
        nth_channel(parts, 0)?,
        nth_channel(parts, 1)?,
        nth_channel(parts, 2)?,
        255,
    ))
}

fn parse_rgb_fn(inner: &str) -> Option<Rgba> {
    let parts: Vec<&str> = inner.split([',', '/']).collect();
    let rgb = parse_rgb_parts(&parts)?;
    let a = match parts.get(3) {
        Some(p) => alpha_channel(p)?,
        None => 255,
    };
    Some(Rgba { a, ..rgb })
}

fn paren_inner(s: &str) -> Option<&str> {
    s.trim().strip_prefix('(')?.strip_suffix(')')
}

fn rgb_inner(t: &str) -> Option<&str> {
    let body = t.strip_prefix("rgba").or_else(|| t.strip_prefix("rgb"))?;
    paren_inner(body)
}

/// Parse a CSS color (`#rgb`/`#rrggbb`/`#rrggbbaa`, `rgb()`/`rgba()`, or a small
/// named set). Returns `None` for an unrecognized value.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(inner) = rgb_inner(t) {
        return parse_rgb_fn(inner);
    }
    #[cfg(feature = "print-color")]
    if let Some(cmyk) = parse_cmyk(t) {
        return Some(cmyk);
    }
    named_color(&t.to_ascii_lowercase())
}

/// Parse a `cmyk(c, m, y, k)` functional colour into the nearest [`Rgba`]
/// (`print-color`, AC-7.x). The four components are percentages or 0..=1
/// fractions. Layout is RGB-internal, so we store the device-equivalent RGB; the
/// emitter (`emit::color::set_fill`) converts it back to DeviceCMYK so the page
/// stream carries CMYK ink. Round-trips exactly for the achromatic and primary
/// inks print stylesheets use.
#[cfg(feature = "print-color")]
fn parse_cmyk(s: &str) -> Option<Rgba> {
    let inner = paren_inner(s.strip_prefix("cmyk")?)?;
    let [c, m, y, k] = cmyk_components(inner)?;
    let to_byte = |ink: f32| ((1.0 - ink) * (1.0 - k) * 255.0).round() as u8;
    Some(Rgba::new(to_byte(c), to_byte(m), to_byte(y), 255))
}

/// Parse the four CMYK components from the `(...)` inner text, or `None` unless
/// exactly four valid components are present.
#[cfg(feature = "print-color")]
fn cmyk_components(inner: &str) -> Option<[f32; 4]> {
    let mut out = [0.0_f32; 4];
    let mut seen = 0;
    for token in inner.split([',', '/']) {
        let slot = out.get_mut(seen)?;
        *slot = cmyk_component(token.trim())?;
        seen += 1;
    }
    (seen == 4).then_some(out)
}

/// One CMYK component: a `NN%` percentage or a bare `0..=1` fraction, clamped.
#[cfg(feature = "print-color")]
fn cmyk_component(token: &str) -> Option<f32> {
    let value = match token.strip_suffix('%') {
        Some(pct) => pct.trim().parse::<f32>().ok()? / 100.0,
        None => token.parse::<f32>().ok()?,
    };
    Some(value.clamp(0.0, 1.0))
}

// --------------------------------------------------------------------------
// resolved box style
// --------------------------------------------------------------------------

/// The fully typed style a box uses for layout, resolved from a [`ComputedStyle`].
#[derive(Debug, Clone, PartialEq)]
pub struct BoxStyle {
    pub display: Display,
    pub position_relative: bool,
    pub margin: Edges,
    pub padding: Edges,
    pub border: BorderEdges,
    pub width: LengthPct,
    pub height: LengthPct,
    pub min_width: LengthPct,
    pub max_width: LengthPct,
    pub min_height: LengthPct,
    pub max_height: LengthPct,
    pub box_sizing: BoxSizing,
    pub font_families: Vec<String>,
    pub font_size: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub color: Rgba,
    pub line_height: Option<f32>,
    pub text_align: crate::text::Align,
    pub white_space: crate::text::WhiteSpace,
    pub letter_spacing: f32,
    pub vertical_align: VAlign,
    pub break_before: BreakRule,
    pub break_after: BreakRule,
    pub break_inside_avoid: bool,
    pub orphans: u8,
    pub widows: u8,
    pub background: Option<Rgba>,
}

/// Context for resolving font-relative and percentage values.
#[derive(Debug, Clone, Copy)]
pub struct ResolveCtx {
    pub parent_font_size: f32,
    pub cb_width: f32,
}

fn resolve_font_size(s: &ComputedStyle, parent: f32) -> f32 {
    let raw = match s.get("font-size") {
        Some(v) => v,
        None => return parent,
    };
    match parse_raw(raw) {
        Some(r) => raw_to_px(r, parent, parent),
        None => parent,
    }
}

/// The resolved `display` keyword (needed at box-generation time, before widths
/// are known).
pub fn display_of(s: &ComputedStyle) -> Display {
    match s.get("display").unwrap_or("block").trim() {
        "inline" => Display::Inline,
        "inline-block" => Display::InlineBlock,
        "flex" => Display::Flex,
        "none" => Display::None,
        "table" => Display::Table,
        "table-row" => Display::TableRow,
        "table-cell" => Display::TableCell,
        "table-header-group" => Display::TableHeaderGroup,
        "table-footer-group" => Display::TableFooterGroup,
        "list-item" => Display::ListItem,
        _ => Display::Block,
    }
}

fn edge_px(token: &str, fs: f32, basis: f32) -> f32 {
    parse_length_pct(token, fs)
        .and_then(|l| l.resolve(basis))
        .unwrap_or(0.0)
}

fn edges_from_slice(v: &[f32]) -> Edges {
    match v {
        [a] => Edges::all(*a),
        [a, b] => Edges {
            top: *a,
            right: *b,
            bottom: *a,
            left: *b,
        },
        [a, b, c] => Edges {
            top: *a,
            right: *b,
            bottom: *c,
            left: *b,
        },
        [a, b, c, d] => Edges {
            top: *a,
            right: *b,
            bottom: *c,
            left: *d,
        },
        _ => Edges::default(),
    }
}

fn parse_edge_shorthand(v: &str, fs: f32, basis: f32) -> Edges {
    let vals: Vec<f32> = v
        .split_whitespace()
        .map(|t| edge_px(t, fs, basis))
        .collect();
    edges_from_slice(&vals)
}

fn side_value(s: &ComputedStyle, prop: &str, fs: f32, basis: f32) -> Option<f32> {
    s.get(prop).map(|v| edge_px(v, fs, basis))
}

fn resolve_edges(s: &ComputedStyle, prefix: &str, fs: f32, basis: f32) -> Edges {
    let mut e = s
        .get(prefix)
        .map(|v| parse_edge_shorthand(v, fs, basis))
        .unwrap_or_default();
    let sides = [
        ("top", &mut e.top),
        ("right", &mut e.right),
        ("bottom", &mut e.bottom),
        ("left", &mut e.left),
    ];
    for (name, slot) in sides {
        if let Some(v) = side_value(s, &format!("{prefix}-{name}"), fs, basis) {
            *slot = v;
        }
    }
    e
}

fn length_prop(s: &ComputedStyle, prop: &str, fs: f32, default: LengthPct) -> LengthPct {
    s.get(prop)
        .and_then(|v| parse_length_pct(v, fs))
        .unwrap_or(default)
}

fn border_width_token(token: &str, fs: f32) -> Option<u16> {
    let w = match token {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        "none" => 0.0,
        other => parse_px(other, fs)?,
    };
    Some(w.round().clamp(0.0, f32::from(u16::MAX)) as u16)
}

fn parse_border_shorthand(v: Option<&str>, fs: f32) -> BorderSide {
    let mut side = BorderSide::default();
    let Some(text) = v else { return side };
    for token in text.split_whitespace() {
        if let Some(w) = border_width_token(token, fs) {
            side.width = w;
        } else if let Some(c) = parse_color(token) {
            side.color = Some(c);
        }
    }
    side
}

fn resolve_border_side(s: &ComputedStyle, name: &str, fs: f32) -> BorderSide {
    let mut b = parse_border_shorthand(s.get("border"), fs);
    if let Some(v) = s.get(&format!("border-{name}")) {
        b = parse_border_shorthand(Some(v), fs);
    }
    if let Some(w) = s
        .get(&format!("border-{name}-width"))
        .and_then(|v| border_width_token(v, fs))
    {
        b.width = w;
    }
    if let Some(c) = s.get(&format!("border-{name}-color")).and_then(parse_color) {
        b.color = Some(c);
    }
    b
}

fn resolve_borders(s: &ComputedStyle, fs: f32) -> BorderEdges {
    BorderEdges {
        top: resolve_border_side(s, "top", fs),
        right: resolve_border_side(s, "right", fs),
        bottom: resolve_border_side(s, "bottom", fs),
        left: resolve_border_side(s, "left", fs),
    }
}

fn font_families(s: &ComputedStyle) -> Vec<String> {
    s.get("font-family")
        .map(|v| {
            v.split(',')
                .map(|f| f.trim().trim_matches(['"', '\'']).to_string())
                .filter(|f| !f.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn font_weight(s: &ComputedStyle) -> u16 {
    match s.get("font-weight").unwrap_or("normal").trim() {
        "normal" => 400,
        "bold" => 700,
        n => n.parse().unwrap_or(400),
    }
}

fn line_height(s: &ComputedStyle, fs: f32) -> Option<f32> {
    let v = s.get("line-height")?.trim();
    if v.eq_ignore_ascii_case("normal") {
        return None;
    }
    match v.parse::<f32>() {
        Ok(mult) => Some(mult * fs),
        Err(_) => parse_px(v, fs),
    }
}

fn align_of(s: &ComputedStyle) -> crate::text::Align {
    use crate::text::Align;
    match s.get("text-align").unwrap_or("left").trim() {
        "right" => Align::Right,
        "center" => Align::Center,
        "justify" => Align::Justify,
        _ => Align::Left,
    }
}

fn white_space_of(s: &ComputedStyle) -> crate::text::WhiteSpace {
    use crate::text::WhiteSpace;
    match s.get("white-space").unwrap_or("normal").trim() {
        "pre" => WhiteSpace::Pre,
        "nowrap" => WhiteSpace::NoWrap,
        _ => WhiteSpace::Normal,
    }
}

fn vertical_align_of(s: &ComputedStyle) -> VAlign {
    match s.get("vertical-align").unwrap_or("baseline").trim() {
        "sub" => VAlign::Sub,
        "super" => VAlign::Super,
        "middle" => VAlign::Middle,
        "top" => VAlign::Top,
        "bottom" => VAlign::Bottom,
        _ => VAlign::Baseline,
    }
}

fn break_rule_of(s: &ComputedStyle, prop: &str) -> BreakRule {
    match s.get(prop).unwrap_or("auto").trim() {
        "avoid" => BreakRule::Avoid,
        "page" | "column" => BreakRule::Page,
        _ => BreakRule::Auto,
    }
}

fn int_prop(s: &ComputedStyle, prop: &str, default: u8) -> u8 {
    s.get(prop)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn resolve_box_metrics(s: &ComputedStyle, fs: f32, ctx: ResolveCtx) -> BoxStyle {
    BoxStyle {
        display: display_of(s),
        position_relative: s.get("position").map(str::trim) == Some("relative"),
        margin: resolve_edges(s, "margin", fs, ctx.cb_width),
        padding: resolve_edges(s, "padding", fs, ctx.cb_width),
        border: resolve_borders(s, fs),
        width: length_prop(s, "width", fs, LengthPct::Auto),
        height: length_prop(s, "height", fs, LengthPct::Auto),
        min_width: length_prop(s, "min-width", fs, LengthPct::Px(0.0)),
        max_width: length_prop(s, "max-width", fs, LengthPct::Auto),
        min_height: length_prop(s, "min-height", fs, LengthPct::Px(0.0)),
        max_height: length_prop(s, "max-height", fs, LengthPct::Auto),
        box_sizing: box_sizing_of(s),
        font_families: font_families(s),
        font_size: fs,
        font_weight: font_weight(s),
        italic: matches!(
            s.get("font-style").map(str::trim),
            Some("italic") | Some("oblique")
        ),
        color: s.get("color").and_then(parse_color).unwrap_or(Rgba::BLACK),
        line_height: line_height(s, fs),
        text_align: align_of(s),
        white_space: white_space_of(s),
        letter_spacing: s
            .get("letter-spacing")
            .and_then(|v| parse_px(v, fs))
            .unwrap_or(0.0),
        vertical_align: vertical_align_of(s),
        break_before: break_rule_of(s, "break-before"),
        break_after: break_rule_of(s, "break-after"),
        break_inside_avoid: s.get("break-inside").map(str::trim) == Some("avoid"),
        orphans: int_prop(s, "orphans", 2),
        widows: int_prop(s, "widows", 2),
        background: s.get("background-color").and_then(parse_color),
    }
}

fn box_sizing_of(s: &ComputedStyle) -> BoxSizing {
    match s.get("box-sizing").map(str::trim) {
        Some("border-box") => BoxSizing::BorderBox,
        _ => BoxSizing::ContentBox,
    }
}

/// Resolve a [`ComputedStyle`] into a typed [`BoxStyle`] for layout.
pub fn resolve_box_style(s: &ComputedStyle, ctx: ResolveCtx) -> BoxStyle {
    crate::hot!("layout.resolve_box_style");
    let fs = resolve_font_size(s, ctx.parent_font_size);
    resolve_box_metrics(s, fs, ctx)
}

/// Whether `resolve_box_style(s, ctx)` is the same for every `ctx` — i.e. the box
/// uses no relative units and has an absolute, present `font-size` (so its font
/// size, and every `em` derived from it, is fixed rather than inherited from the
/// context). When true a box may resolve its style once and reuse it across the
/// measure and placement passes; otherwise it must re-resolve per call.
pub(crate) fn is_ctx_independent(s: &ComputedStyle) -> bool {
    s.has_no_relative_units() && font_size_absolute(s)
}

fn font_size_absolute(s: &ComputedStyle) -> bool {
    matches!(
        s.get("font-size").and_then(parse_raw),
        Some(RawLength::Abs(_))
    )
}

//! Box generation (§5.1): the styled tree becomes a box tree. CSS distinguishes
//! block-level and inline-level boxes; a block container holding a mix wraps each
//! inline run in an *anonymous block* so its children are uniformly block-level
//! (AC-5.1). `display:none` is dropped; `t:` directives become opaque markers the
//! fragmenter/emitter handle later.
//!
//! Boxes keep their [`ComputedStyle`] rather than a resolved `BoxStyle`: `%`
//! widths/margins need a containing-block width that is only known during layout,
//! so metric resolution is deferred to the block/inline/flex/table passes. Each
//! box gets a pre-order [`NodeId`] for round-tripping back to its template node.

use std::cell::RefCell;

use crate::node::{Attr, TKind, Tag};
use crate::style::{ComputedStyle, StyledElement, StyledNode};

use super::fragment::NodeId;
use super::value::{
    display_of, is_ctx_independent, resolve_box_style, BoxStyle, Display, ResolveCtx,
};

/// A box in the layout tree.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node_id: NodeId,
    pub style: ComputedStyle,
    /// Source element attributes (e.g. `colspan`, `href`); empty for anonymous
    /// boxes. Layout/emit read HTML attributes that are not CSS properties here.
    pub attrs: Vec<Attr>,
    pub display: Display,
    pub kind: BoxKind,
    /// A raster image this box paints (§7.4): a replaced `<img>` (which also
    /// sizes the box) or a `background-image` (painted behind the box content).
    pub image: Option<ImageSource>,
    /// A `mask-image: url(...)` (or `-webkit-mask-image`): the icon technique where
    /// an SVG is used as an alpha mask and the box's paint (its `background-color`,
    /// falling back to `color`) shows through only where the mask is opaque. Ubiquitous
    /// for Wikipedia's UI glyphs (TOC carets, search/eye/pencil). The box paints as a
    /// tinted mask instead of a solid rectangle.
    pub mask: Option<String>,
    /// Memoized style resolution. Layout resolves each box's `BoxStyle` several
    /// times (max-content measurement, then placement); when the metrics cannot
    /// vary with the containing block, the first resolution is reused instead of
    /// re-parsing the ~25 properties. The (one-time) independence classification
    /// is also cached so context-*dependent* boxes never re-scan their values.
    style_cache: RefCell<StyleCache>,
    /// Memoized intrinsic widths (max-content, min-content). Both are pure
    /// functions of the box subtree + fonts (no layout context), but the flex/table
    /// sizers call them repeatedly and RE-descend the subtree each time, so a deeply
    /// nested tree measured naively is superlinear. Cache the first result per box.
    natural_cache: std::cell::Cell<Option<f32>>,
    min_content_cache: std::cell::Cell<Option<f32>>,
    /// A replaced `<img>`'s intrinsic pixel width, stamped once from the image
    /// resolver before layout ([`crate::layout::stamp_intrinsic_widths`]). The
    /// intrinsic size needs the decoded bytes (only the layout entry has the
    /// resolver), but `natural_width`/`min_content_width` — which run deep in the
    /// flex/table sizers without a resolver — must see it so a `<div>` wrapping an
    /// `<img>` (a logo) measures the image's width instead of collapsing to 0.
    pub(crate) intrinsic_w: std::cell::Cell<Option<f32>>,
    /// Memoized flex/grid measurement: the content `(width, height)` this box lays
    /// out to at a given proposed width. taffy probes each item's size several times
    /// per solve (min/max-content, then the resolved width) and each probe does a
    /// FULL sub-layout, so a deeply nested flex tree is exponential without this —
    /// keyed by the rounded proposed width (a handful of distinct values per box).
    measure_cache: RefCell<Vec<(u32, (f32, f32))>>,
    /// The PDF/UA structure role derived from this box's HTML tag (`pdf-ua`),
    /// carried down to the fragment so the emitter can tag it (AC-11.1).
    #[cfg(feature = "pdf-ua")]
    pub ua_role: Option<crate::layout::fragment::UaRole>,
    /// Alternate text for an `<img>` (`alt` attribute), written as `/Alt` on the
    /// figure's struct element (`pdf-ua`).
    #[cfg(feature = "pdf-ua")]
    pub ua_alt: Option<String>,
}

/// Per-box cache of the resolved style and its context-independence verdict.
#[derive(Debug, Clone)]
enum StyleCache {
    /// Not yet classified — resolve and decide on first use.
    Unknown,
    /// Style depends on the layout context; always re-resolve (no value cached).
    Dependent,
    /// Style is context-independent; this resolution is reused for every `ctx`.
    /// Boxed to keep the enum small (a `BoxStyle` dwarfs the unit variants).
    Cached(Box<BoxStyle>),
}

impl LayoutBox {
    /// The memoized max-content width, computing it once via `f` on a miss.
    pub(crate) fn natural_cached(&self, f: impl FnOnce() -> f32) -> f32 {
        match self.natural_cache.get() {
            Some(w) => w,
            None => {
                let w = f();
                self.natural_cache.set(Some(w));
                w
            }
        }
    }

    /// The memoized min-content width, computing it once via `f` on a miss.
    pub(crate) fn min_content_cached(&self, f: impl FnOnce() -> f32) -> f32 {
        match self.min_content_cache.get() {
            Some(w) => w,
            None => {
                let w = f();
                self.min_content_cache.set(Some(w));
                w
            }
        }
    }

    /// The memoized measured content size at proposed width `w`, computing it once
    /// per distinct (rounded) width via `f`.
    pub(crate) fn measure_cached(&self, w: f32, f: impl FnOnce() -> (f32, f32)) -> (f32, f32) {
        let key = w.round().clamp(0.0, u32::MAX as f32) as u32;
        // Scope the immutable borrow to this `let` so it is dropped before `f()` (a
        // recursive sub-layout) or the `borrow_mut` below can run.
        let hit = self
            .measure_cache
            .borrow()
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, s)| *s);
        if let Some(size) = hit {
            return size;
        }
        let size = f();
        self.measure_cache.borrow_mut().push((key, size));
        size
    }

    /// Resolve this box's [`BoxStyle`] for `ctx`, reusing the cached resolution
    /// when the style is context-independent.
    pub(crate) fn resolved(&self, ctx: ResolveCtx) -> BoxStyle {
        match &*self.style_cache.borrow() {
            StyleCache::Cached(bs) => return bs.as_ref().clone(),
            StyleCache::Dependent => return resolve_box_style(&self.style, ctx),
            StyleCache::Unknown => {}
        }
        self.resolve_and_classify(ctx)
    }

    /// First-use path: resolve once, then remember whether the result can be
    /// reused for any context.
    fn resolve_and_classify(&self, ctx: ResolveCtx) -> BoxStyle {
        let bs = resolve_box_style(&self.style, ctx);
        *self.style_cache.borrow_mut() = if is_ctx_independent(&self.style) {
            StyleCache::Cached(Box::new(bs.clone()))
        } else {
            StyleCache::Dependent
        };
        bs
    }
}

/// A raster image referenced by a box: the resolver name plus whether it is the
/// box's *replaced content* (an `<img>`, which drives the box size) or a
/// `background-image` (sized by the box, painted behind it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSource {
    pub name: String,
    pub replaced: bool,
}

/// What a box contains, by formatting context.
#[derive(Debug, Clone)]
pub enum BoxKind {
    /// A block container whose children are all block-level (anonymous blocks
    /// already inserted around inline runs).
    Block(Vec<LayoutBox>),
    /// A block container establishing an inline formatting context: a paragraph
    /// of inline items laid out into line boxes.
    Lines(Vec<InlineItem>),
    /// A flex container; children are flex items.
    Flex(Vec<LayoutBox>),
    /// A grid container; children are grid items (auto-placed unless they carry
    /// explicit `grid-row`/`grid-column` lines).
    Grid(Vec<LayoutBox>),
    /// A table; children are rows/row-groups (interpreted by `table.rs` via
    /// each child's `display`).
    Table(Vec<LayoutBox>),
    /// An opaque paged-media directive marker.
    Directive(TKind),
}

/// One inline-level item inside a [`BoxKind::Lines`] context.
#[derive(Debug, Clone)]
pub enum InlineItem {
    /// A run of text, styled by its containing element.
    Text {
        node_id: NodeId,
        style: ComputedStyle,
        text: String,
    },
    /// An atomic inline box (`inline-block`, or a block nested in inline flow).
    Atomic(LayoutBox),
    /// A forced line break (`<br>`): it ends the current line and continues on the
    /// next (browser-faithful inline behavior). It carries no metrics of its own —
    /// the line-height in effect comes from the line's content and the block strut.
    LineBreak,
    /// An inline paged-media directive (e.g. a footnote reference).
    Directive {
        node_id: NodeId,
        kind: TKind,
        /// A `<t:anchor name>`'s destination name (`xref` feature, AC-3.25),
        /// carried so the positioned directive fragment can define the dest.
        #[cfg(feature = "xref")]
        anchor: Option<String>,
    },
}

/// A block-level box or an inline-level run, before anonymous-block wrapping.
// A transient classification value moved straight into the box tree; the big
// `Block` variant holds the actual `LayoutBox`, so boxing it would just add an
// allocation on the hot construction path for no real gain.
#[allow(clippy::large_enum_variant)]
enum Level {
    Block(LayoutBox),
    Inline(Vec<InlineItem>),
}

/// A monotonic source of pre-order node ids.
struct Ids {
    next: u32,
}

impl Ids {
    fn alloc(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }
}

/// `t:` directives that sit inline within text flow (vs. block-level ones).
fn inline_directive(kind: TKind) -> bool {
    matches!(
        kind,
        TKind::Footnote
            | TKind::Page
            | TKind::Pages
            | TKind::Counter
            | TKind::Leader
            | TKind::Anchor
    )
}

fn is_directive(el: &StyledElement) -> bool {
    matches!(el.tag, Tag::Directive(_))
}

fn text_item(text: &str, style: &ComputedStyle, ids: &mut Ids) -> InlineItem {
    InlineItem::Text {
        node_id: ids.alloc(),
        style: style.clone(),
        text: text.to_string(),
    }
}

// --------------------------------------------------------------------------
// classification
// --------------------------------------------------------------------------

fn directive_level(kind: TKind, el: &StyledElement, ids: &mut Ids) -> Level {
    if inline_directive(kind) {
        Level::Inline(vec![InlineItem::Directive {
            node_id: ids.alloc(),
            kind,
            #[cfg(feature = "xref")]
            anchor: xref::anchor_name(kind, el),
        }])
    } else {
        Level::Block(build_block_box(el, ids))
    }
}

/// Whether a box (and its subtree) is not rendered: `display:none`, or the
/// invisible states `visibility:hidden` / `opacity:0`. We drop it rather than
/// paint an invisible box — which is also what keeps hover/click-revealed menus
/// (Wikipedia's nav dropdowns) hidden in a static snapshot, since their reveal
/// rule (`:hover`/`:checked ~ …`) is never applied.
pub(crate) fn is_hidden(style: &ComputedStyle) -> bool {
    if matches!(display_of(style), Display::None) {
        return true;
    }
    if style.get("visibility").map(str::trim) == Some("hidden") {
        return true;
    }
    if matches!(
        style.get("opacity").map(|v| v.trim().parse::<f32>()),
        Some(Ok(o)) if o <= 0.0
    ) {
        return true;
    }
    is_visually_hidden(style)
}

/// The "visually hidden" / screen-reader-only patterns: content that's in the DOM
/// (for a11y) but clipped away so it never paints. Real pages (Wikipedia, Nike)
/// use dozens; without honoring them their text renders — and, being
/// `position:absolute` with no offset, piles up at the containing block's origin.
fn is_visually_hidden(style: &ComputedStyle) -> bool {
    // `clip: rect(...)` — deprecated everywhere *except* the classic sr-only clip
    // hack (`clip:rect(1px,1px,1px,1px)` / `rect(0,0,0,0)`), so any rect() clip is
    // the hide pattern.
    if style
        .get("clip")
        .map(str::trim)
        .is_some_and(|c| c.starts_with("rect("))
    {
        return true;
    }
    clip_path_hides(style) || tiny_clipped(style)
}

/// `clip-path: inset(50%|100%)` clips the whole box away.
fn clip_path_hides(style: &ComputedStyle) -> bool {
    style
        .get("clip-path")
        .map(str::trim)
        .is_some_and(|cp| cp.contains("inset(") && (cp.contains("50%") || cp.contains("100%")))
}

/// A 0/1px box with clipped overflow — the modern visually-hidden pattern.
fn tiny_clipped(style: &ComputedStyle) -> bool {
    let tiny = |p: &str| {
        matches!(
            style.get(p).map(str::trim),
            Some("0") | Some("0px") | Some("1px")
        )
    };
    let overflow_hidden = ["overflow", "overflow-x", "overflow-y"]
        .iter()
        .any(|p| style.get(p).map(str::trim) == Some("hidden"));
    tiny("width") && tiny("height") && overflow_hidden
}

/// Whether the element is a `<br>` — an inline forced line break, handled by the
/// inline layout, not a box (it has no `display`/`height` of its own).
fn is_br(el: &StyledElement) -> bool {
    matches!(&el.tag, Tag::Html(n) if n == "br")
}

fn classify_html(el: &StyledElement, ids: &mut Ids) -> Option<Level> {
    if is_hidden(&el.style) {
        return None;
    }
    if is_br(el) {
        return Some(Level::Inline(vec![InlineItem::LineBreak]));
    }
    #[cfg(feature = "xref")]
    if xref::internal_link_href(el).is_some() {
        // An `<a href="#name">` is laid out as an atomic inline box so it carries
        // its own fragment (and thus a link rectangle) through layout (AC-3.25).
        return Some(Level::Inline(vec![InlineItem::Atomic(build_block_box(
            el, ids,
        ))]));
    }
    // `display:none` was already dropped by the `is_hidden` guard above, so it can
    // never reach here — no `Display::None` arm (it would be dead code).
    match display_of(&el.style) {
        Display::Inline => Some(Level::Inline(flatten_inline(el, ids))),
        Display::InlineBlock => Some(Level::Inline(vec![InlineItem::Atomic(build_block_box(
            el, ids,
        ))])),
        _ => Some(Level::Block(build_block_box(el, ids))),
    }
}

fn classify(node: &StyledNode, parent_style: &ComputedStyle, ids: &mut Ids) -> Option<Level> {
    let el = match node {
        StyledNode::Text(t) => return Some(Level::Inline(vec![text_item(t, parent_style, ids)])),
        StyledNode::Element(e) => e,
    };
    match &el.tag {
        Tag::Directive(kind) => Some(directive_level(*kind, el, ids)),
        Tag::Html(_) => classify_html(el, ids),
    }
}

/// Flatten an inline element's content into inline items (its own style applies;
/// a nested block becomes an atomic inline).
fn flatten_inline(el: &StyledElement, ids: &mut Ids) -> Vec<InlineItem> {
    let mut out = Vec::new();
    for child in &el.children {
        match classify(child, &el.style, ids) {
            Some(Level::Inline(items)) => out.extend(items),
            Some(Level::Block(b)) => out.push(InlineItem::Atomic(b)),
            None => {}
        }
    }
    out
}

// --------------------------------------------------------------------------
// block-box construction + anonymous wrapping
// --------------------------------------------------------------------------

fn box_kind_for(display: Display, el: &StyledElement, ids: &mut Ids) -> BoxKind {
    // A button `<input type="submit|button|reset">` carries its label in the `value`
    // ATTRIBUTE, not as text content — render it so the box isn't empty (google's
    // "Google Search" / "I'm Feeling Lucky" buttons were blank grey rectangles).
    if let Some(label) = input_button_text(el) {
        let text = vec![StyledNode::Text(label)];
        return build_flow(&text, &el.style, ids);
    }
    match display {
        Display::Flex => BoxKind::Flex(child_block_boxes(el, ids)),
        Display::Grid => BoxKind::Grid(child_block_boxes(el, ids)),
        Display::Table => BoxKind::Table(child_block_boxes(el, ids)),
        _ => build_flow(&el.children, &el.style, ids),
    }
}

/// The label of a button-like `<input>` (`type` submit/button/reset) — its `value`
/// attribute, shown as the box's text. `None` for any other element/input type.
fn input_button_text(el: &StyledElement) -> Option<String> {
    if !matches!(&el.tag, Tag::Html(n) if n == "input") {
        return None;
    }
    let ty = attr_value(&el.attrs, "type").unwrap_or("text");
    matches!(ty, "submit" | "button" | "reset")
        .then(|| attr_value(&el.attrs, "value").map(str::to_string))
        .flatten()
}

fn build_block_box(el: &StyledElement, ids: &mut Ids) -> LayoutBox {
    let node_id = ids.alloc();
    if let Tag::Directive(kind) = &el.tag {
        return LayoutBox {
            node_id,
            style: el.style.clone(),
            attrs: el.attrs.clone(),
            display: Display::Block,
            kind: BoxKind::Directive(*kind),
            image: None,
            mask: None,
            style_cache: RefCell::new(StyleCache::Unknown),
            natural_cache: std::cell::Cell::new(None),
            min_content_cache: std::cell::Cell::new(None),
            intrinsic_w: std::cell::Cell::new(None),
            measure_cache: RefCell::new(Vec::new()),
            #[cfg(feature = "pdf-ua")]
            ua_role: None,
            #[cfg(feature = "pdf-ua")]
            ua_alt: None,
        };
    }
    let display = display_of(&el.style);
    let kind = box_kind_for(display, el, ids);
    LayoutBox {
        node_id,
        style: el.style.clone(),
        attrs: el.attrs.clone(),
        display,
        kind,
        image: image_of(el),
        mask: mask_image(&el.style),
        style_cache: RefCell::new(StyleCache::Unknown),
        natural_cache: std::cell::Cell::new(None),
        min_content_cache: std::cell::Cell::new(None),
        intrinsic_w: std::cell::Cell::new(None),
        measure_cache: RefCell::new(Vec::new()),
        #[cfg(feature = "pdf-ua")]
        ua_role: ua::role_of(el),
        #[cfg(feature = "pdf-ua")]
        ua_alt: ua::alt_of(el),
    }
}

/// The raster image a styled element references: an `<img src>` is replaced
/// content; a `background-image: url(...)` paints behind the box. The `<img>`
/// source wins when both are present.
fn image_of(el: &StyledElement) -> Option<ImageSource> {
    if let Some(src) = img_src(el) {
        return Some(ImageSource {
            name: src.to_string(),
            replaced: true,
        });
    }
    background_image(&el.style).map(|name| ImageSource {
        name,
        replaced: false,
    })
}

/// The image URL an `<img>` paints, or `None` for any other tag. Prefers a plain
/// `src`, then a lazy-load data attribute, then the first `srcset` candidate.
fn img_src(el: &StyledElement) -> Option<&str> {
    match &el.tag {
        Tag::Html(name) if name == "img" => img_url_attr(&el.attrs),
        _ => None,
    }
}

/// The painted URL among an `<img>`'s attributes: `src` first, else a lazy-load
/// data attribute (`data-src`/`data-landscape-url`/…), else the first `srcset`
/// candidate. Nike's hero `<img>`s ship no `src`/`srcset` — the real URL lives in
/// `data-landscape-url` (client JS copies it to `src` on scroll) — so without this
/// fallback every hero image laid out as an empty box (a blank hero band).
fn img_url_attr(attrs: &[Attr]) -> Option<&str> {
    const KEYS: [&str; 6] = [
        "src",
        "data-src",
        "data-landscape-url",
        "data-portrait-url",
        "data-original",
        "data-image-src",
    ];
    for k in KEYS {
        if let Some(v) = attr_value(attrs, k).filter(|v| !v.trim().is_empty()) {
            return Some(v);
        }
    }
    attr_value(attrs, "srcset").and_then(srcset_first_url)
}

/// The URL of a `srcset`'s first candidate (`url [descriptor]`, comma-separated).
fn srcset_first_url(srcset: &str) -> Option<&str> {
    srcset
        .split(',')
        .next()
        .and_then(|c| c.split_whitespace().next())
}

/// The value of the named attribute in `attrs`, if present.
fn attr_value<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}

/// The url of a `background-image: url(...)` longhand, or of a `url(...)` in the
/// `background` shorthand (real stylesheets set images via the shorthand). A
/// `none`/gradient/unparsable value yields `None`.
fn background_image(style: &ComputedStyle) -> Option<String> {
    if let Some(url) = style.get("background-image").and_then(url_token) {
        return Some(url);
    }
    let bg = style.get("background")?;
    super::value::css_value_tokens(bg)
        .into_iter()
        .find_map(url_token)
}

/// The url of a `mask-image`/`-webkit-mask-image: url(...)` (or a `url(...)` in the
/// `mask` shorthand). Icons use this with `background-color` to tint an SVG glyph.
fn mask_image(style: &ComputedStyle) -> Option<String> {
    for prop in ["mask-image", "-webkit-mask-image", "mask", "-webkit-mask"] {
        let Some(v) = style.get(prop).map(str::trim) else {
            continue;
        };
        // A single `url(...)` may be a `data:` URI whose body contains commas (an
        // inline `<svg>` mask) — take it whole rather than splitting on commas.
        if let Some(url) = url_token(v) {
            return Some(url);
        }
        if let Some(url) = super::value::css_value_tokens(v)
            .into_iter()
            .find_map(url_token)
        {
            return Some(url);
        }
    }
    None
}

/// The bare url inside a `url(...)` token (quotes stripped), or `None` for any
/// other token.
fn url_token(token: &str) -> Option<String> {
    let inner = token.trim().strip_prefix("url(")?.strip_suffix(')')?;
    let name = inner.trim().trim_matches(['"', '\'']).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Build a flex/table container's children as block-level boxes (raw text and
/// `display:none` dropped; v1 does not synthesize anonymous flex/table items).
fn child_block_boxes(el: &StyledElement, ids: &mut Ids) -> Vec<LayoutBox> {
    el.children
        .iter()
        .filter_map(|n| block_child(n, ids))
        .collect()
}

fn block_child(node: &StyledNode, ids: &mut Ids) -> Option<LayoutBox> {
    let el = node.as_element()?;
    if !is_directive(el) && is_hidden(&el.style) {
        return None;
    }
    Some(build_block_box(el, ids))
}

fn run_is_blank(run: &[InlineItem]) -> bool {
    run.iter().all(is_blank_text)
}

fn is_blank_text(item: &InlineItem) -> bool {
    matches!(item, InlineItem::Text { text, .. } if text.trim().is_empty())
}

fn anon_lines_box(
    items: Vec<InlineItem>,
    parent_style: &ComputedStyle,
    ids: &mut Ids,
) -> LayoutBox {
    LayoutBox {
        node_id: ids.alloc(),
        style: parent_style.clone(),
        attrs: Vec::new(),
        display: Display::Block,
        kind: BoxKind::Lines(items),
        image: None,
        mask: None,
        style_cache: RefCell::new(StyleCache::Unknown),
        natural_cache: std::cell::Cell::new(None),
        min_content_cache: std::cell::Cell::new(None),
        intrinsic_w: std::cell::Cell::new(None),
        measure_cache: RefCell::new(Vec::new()),
        // An anonymous block wrapping an inline run reads as a paragraph of text.
        #[cfg(feature = "pdf-ua")]
        ua_role: Some(crate::layout::fragment::UaRole::Paragraph),
        #[cfg(feature = "pdf-ua")]
        ua_alt: None,
    }
}

fn flush_run(
    run: &mut Vec<InlineItem>,
    parent_style: &ComputedStyle,
    ids: &mut Ids,
    out: &mut Vec<LayoutBox>,
) {
    if run.is_empty() || run_is_blank(run) {
        run.clear();
        return;
    }
    let items = std::mem::take(run);
    out.push(anon_lines_box(items, parent_style, ids));
}

fn wrap_runs(levels: Vec<Level>, parent_style: &ComputedStyle, ids: &mut Ids) -> Vec<LayoutBox> {
    let mut out = Vec::new();
    let mut run: Vec<InlineItem> = Vec::new();
    for level in levels {
        match level {
            Level::Inline(items) => run.extend(items),
            Level::Block(b) => {
                flush_run(&mut run, parent_style, ids, &mut out);
                out.push(b);
            }
        }
    }
    flush_run(&mut run, parent_style, ids, &mut out);
    out
}

fn inline_items_of(levels: Vec<Level>) -> Vec<InlineItem> {
    // Only reached when no level is block-level, so every level is `Inline`.
    let mut out = Vec::new();
    for level in levels {
        if let Level::Inline(items) = level {
            out.extend(items);
        }
    }
    out
}

/// Build the formatting context for a flow of children: a block context (with
/// anonymous-block wrapping) if any child is block-level, else an inline context.
fn build_flow(children: &[StyledNode], parent_style: &ComputedStyle, ids: &mut Ids) -> BoxKind {
    let levels: Vec<Level> = children
        .iter()
        .filter_map(|n| classify(n, parent_style, ids))
        .collect();
    if levels.iter().any(|l| matches!(l, Level::Block(_))) {
        BoxKind::Block(wrap_runs(levels, parent_style, ids))
    } else {
        BoxKind::Lines(inline_items_of(levels))
    }
}

/// Build the box tree for a document flow. The root is an anonymous block box
/// (id 0) whose `kind` is the top-level formatting context.
pub fn build_box_tree(styled: &[StyledNode]) -> LayoutBox {
    let mut ids = Ids { next: 0 };
    let node_id = ids.alloc();
    let style = ComputedStyle::default();
    let kind = build_flow(styled, &style, &mut ids);
    LayoutBox {
        node_id,
        style,
        attrs: Vec::new(),
        display: Display::Block,
        kind,
        image: None,
        mask: None,
        style_cache: RefCell::new(StyleCache::Unknown),
        natural_cache: std::cell::Cell::new(None),
        min_content_cache: std::cell::Cell::new(None),
        intrinsic_w: std::cell::Cell::new(None),
        measure_cache: RefCell::new(Vec::new()),
        // The synthetic document root maps to the `Document` structure element.
        #[cfg(feature = "pdf-ua")]
        ua_role: Some(crate::layout::fragment::UaRole::Group),
        #[cfg(feature = "pdf-ua")]
        ua_alt: None,
    }
}

/// PDF/UA role derivation from the semantic HTML tag (`pdf-ua` feature, AC-11.1).
/// The gated-only body lives in its own module file so its branches stay out of
/// the default coverage surface; exercised by the `--features pdf-ua` tests.
#[cfg(feature = "pdf-ua")]
#[path = "boxgen_ua.rs"]
mod ua;

/// `xref`-feature box-generation helpers (anchor names + internal link hrefs,
/// AC-3.25). The gated-only body lives in its own module file so its branches
/// stay out of the default coverage surface; exercised by `--features xref`.
#[cfg(feature = "xref")]
#[path = "boxgen_xref.rs"]
mod xref;

#[cfg(test)]
mod tests {
    use super::*;

    fn html_el(tag: &str, children: Vec<StyledNode>) -> StyledNode {
        StyledNode::Element(StyledElement {
            tag: Tag::Html(tag.to_string()),
            attrs: vec![],
            style: ComputedStyle::default(),
            children,
        })
    }

    fn bare(tag: &str) -> StyledElement {
        StyledElement {
            tag: Tag::Html(tag.to_string()),
            attrs: vec![],
            style: ComputedStyle::default(),
            children: vec![],
        }
    }

    #[test]
    fn is_br_matches_only_the_br_tag() {
        assert!(is_br(&bare("br")));
        assert!(!is_br(&bare("span")));
    }

    #[test]
    fn br_becomes_an_inline_line_break() {
        // `<div>a<br>b</div>`: the <br> is an inline forced break within the div's
        // inline formatting context (a `LineBreak` item), not a block box.
        let tree = build_box_tree(&[html_el(
            "div",
            vec![
                StyledNode::Text("a".to_string()),
                html_el("br", vec![]),
                StyledNode::Text("b".to_string()),
            ],
        )]);
        let BoxKind::Block(kids) = &tree.kind else {
            panic!("root is a block");
        };
        let BoxKind::Lines(items) = &kids[0].kind else {
            panic!("div establishes an inline (lines) context");
        };
        assert!(
            items.iter().any(|i| matches!(i, InlineItem::LineBreak)),
            "the <br> produced a LineBreak item"
        );
    }
}

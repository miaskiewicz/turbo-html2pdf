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

use crate::node::{Attr, TKind, Tag};
use crate::style::{ComputedStyle, StyledElement, StyledNode};

use super::fragment::NodeId;
use super::value::{display_of, Display};

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
    /// An inline paged-media directive (e.g. a footnote reference).
    Directive { node_id: NodeId, kind: TKind },
}

/// A block-level box or an inline-level run, before anonymous-block wrapping.
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
        }])
    } else {
        Level::Block(build_block_box(el, ids))
    }
}

fn classify_html(el: &StyledElement, ids: &mut Ids) -> Option<Level> {
    match display_of(&el.style) {
        Display::None => None,
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
    match display {
        Display::Flex => BoxKind::Flex(child_block_boxes(el, ids)),
        Display::Table => BoxKind::Table(child_block_boxes(el, ids)),
        _ => build_flow(&el.children, &el.style, ids),
    }
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

/// The `src` of an `<img>` element, or `None` for any other tag.
fn img_src(el: &StyledElement) -> Option<&str> {
    match &el.tag {
        Tag::Html(name) if name == "img" => attr_value(&el.attrs, "src"),
        _ => None,
    }
}

/// The value of the named attribute in `attrs`, if present.
fn attr_value<'a>(attrs: &'a [Attr], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.value.as_str())
}

/// The url of a `background-image: url(...)` declaration, or `None`. A
/// `background-image: none` (or unparsable value) yields `None`.
fn background_image(style: &ComputedStyle) -> Option<String> {
    let value = style.get("background-image")?.trim();
    let inner = value.strip_prefix("url(")?.strip_suffix(')')?;
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
    if !is_directive(el) && matches!(display_of(&el.style), Display::None) {
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
    }
}

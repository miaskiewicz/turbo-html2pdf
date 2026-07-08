//! The galley: layout's output (§5.5). A [`Fragment`] tree in one continuous
//! top-down coordinate space (y grows downward, height unbounded). Each fragment
//! carries its source [`NodeId`] so emitted PDF maps back to template nodes
//! (AC-5.11), its painted [`FragmentContent`], and [`BreakMeta`] the fragmenter
//! (Stage 4) reads to decide page breaks.
//!
//! Deferred to later phases (added when first populated): the footnote/running-
//! element payloads on `BreakMeta` (Phases 7/8).

use crate::node::TKind;
use crate::text::FontFace;

use super::value::{BorderEdges, BreakRule, Rgba};

/// A stable identifier round-tripping a fragment to its source template node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NodeId(pub u32);

/// A PDF/UA structure role carried from the semantic HTML tag down to the
/// fragment so the tagged-PDF emitter (`pdf-ua` feature) can build a
/// `StructTreeRoot`. Mirrors the subset of ISO 32000 standard structure types
/// this engine maps to; the emitter translates it to a `pdf_writer::StructRole`.
///
/// Gated to the `pdf-ua` feature so the default build carries no extra field on
/// every fragment (zero cost when the feature is off, AC-11.1).
#[cfg(feature = "pdf-ua")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UaRole {
    /// A structural container with no own marked content (`<div>`, `<section>`).
    Group,
    /// `<h1>`..`<h6>`, by 1-based level.
    Heading(u8),
    /// A paragraph (`<p>` and block text).
    Paragraph,
    /// A list (`<ul>`/`<ol>`).
    List,
    /// A list item (`<li>`).
    ListItem,
    /// A list item's body (the content of an `<li>`).
    ListBody,
    /// A table (`<table>`).
    Table,
    /// A table row (`<tr>`).
    TableRow,
    /// A table header cell (`<th>`).
    TableHeader,
    /// A table data cell (`<td>`).
    TableData,
    /// A figure (`<img>`); carries `/Alt` from the `alt` attribute.
    Figure,
    /// A run of inline text (`<span>`, `<a>`, anonymous inline boxes).
    Span,
    /// Page decoration to be skipped by assistive tech (`/Artifact`): running
    /// header/footer chrome and watermarks. Not a structure element.
    Artifact,
}

/// One shaped glyph, positioned relative to its `TextLine` fragment's origin so
/// shifting the fragment (e.g. during pagination) never rewrites glyph offsets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    pub glyph_id: u16,
    pub x: f32,
    pub y: f32,
}

/// A repeatable table part: re-emitted atop each page a table spans (§6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatKind {
    Header,
    Footer,
}

/// What a fragment paints.
#[derive(Debug, Clone)]
pub enum FragmentContent {
    /// A block/box rectangle: background fill and/or border.
    Box {
        background: Option<Rgba>,
        border: BorderEdges,
        /// `border-radius` resolved to px (0 = square corners). Clamped to half the
        /// box's shorter side at layout, so `50%` on a square is a full circle.
        border_radius: f32,
        /// `box-shadow` (first/topmost layer), painted behind the box by the raster
        /// so overlay cards/modals read as raised chrome. `None` = no shadow.
        shadow: Option<BoxShadow>,
    },
    /// One laid-out line of shaped text.
    TextLine {
        glyphs: Vec<PositionedGlyph>,
        face: FontFace,
        font_size: f32,
        color: Rgba,
    },
    /// A paged-media directive kept opaque for the fragmenter/emitter.
    Directive(TKind),
    /// A raster image (`<img>` or `background-image`, §7.4). The fragment's
    /// `width`/`height` are the painted size (already overflow-capped at layout);
    /// the variant carries only what the emitter needs beyond geometry: the
    /// resolver `name` to fetch the bytes again at emit time, the source's
    /// intrinsic pixel size (diagnostic / round-trip), and whether it carries an
    /// alpha channel (so an SMask is emitted).
    Image(ImagePlacement),
}

/// A resolved `box-shadow` layer (§CSS backgrounds §7). Geometry is px in the
/// galley's coordinate space; the raster stamps a rounded rect of `color` at the
/// box's outline, offset by `(offset_x, offset_y)`, grown by `spread`, blurred by
/// `blur`. `inset` shadows paint inside the box (v1 paints outer only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub spread: i32,
    pub color: Rgba,
    pub inset: bool,
}

/// A placed raster image: the resolver key plus its intrinsic pixel size and
/// alpha flag (§7.4). The pixels themselves are re-resolved at emit time so the
/// galley stays small and the same bytes feed both layout sizing and embedding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacement {
    /// The image name passed to the caller's `ImageResolver` (the `<img>` `src`
    /// or the `background-image` `url(...)`).
    pub name: String,
    /// Intrinsic width in source pixels.
    pub intrinsic_w: u32,
    /// Intrinsic height in source pixels.
    pub intrinsic_h: u32,
    /// Whether the source had transparency (drives SMask emission).
    pub has_alpha: bool,
    /// When set, the image is a CSS `mask-image`: paint this solid colour everywhere
    /// the source is opaque (using the source's alpha as a stencil) rather than
    /// blitting the source pixels. Tints Wikipedia's monochrome SVG UI glyphs.
    pub tint: Option<Rgba>,
}

/// Break hints attached to a fragment, consumed by the fragmenter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakMeta {
    pub break_before: BreakRule,
    pub break_after: BreakRule,
    pub break_inside_avoid: bool,
    pub orphans: u8,
    pub widows: u8,
    pub repeatable: Option<RepeatKind>,
    /// Indices into the document's footnote list owned by this fragment (§6.4):
    /// the footnote markers it carries, in document order. The fragmenter reads
    /// this to learn which notes a page references once the fragment lands, so it
    /// can reserve their measured area. Empty for everything but footnote markers.
    pub footnotes: Vec<usize>,
}

impl Default for BreakMeta {
    fn default() -> Self {
        BreakMeta {
            break_before: BreakRule::Auto,
            break_after: BreakRule::Auto,
            break_inside_avoid: false,
            orphans: 2,
            widows: 2,
            repeatable: None,
            footnotes: Vec::new(),
        }
    }
}

/// A positioned, sized fragment in the galley.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub node_id: NodeId,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub content: FragmentContent,
    pub break_meta: BreakMeta,
    pub children: Vec<Fragment>,
    /// The box's used `z-index` (`None` = `auto`), meaningful only when
    /// [`Fragment::is_positioned`]. Drives stacking-context paint order (§9.9);
    /// see [`Fragment::paint_z`]/[`Fragment::paint_order`].
    pub z_index: Option<i32>,
    /// Whether this fragment's box is positioned (`position` != `static`). A
    /// positioned box paints above its in-flow siblings within the same stacking
    /// context, and (with a `z-index`) participates in z ordering.
    pub is_positioned: bool,
    /// Cross-reference payload (`xref` feature, AC-3.25): the destination name
    /// of a `<t:anchor name>` that landed here, and/or the `#fragment` target of
    /// an `<a href>` whose box this is. Only present under `--features xref`; the
    /// default build does not carry these fields, so its galley is unchanged.
    #[cfg(feature = "xref")]
    pub xref: XrefMeta,
    /// The PDF/UA structure role this fragment carries (`pdf-ua` feature). `None`
    /// means the fragment is transparent to tagging — its marked content attaches
    /// to the nearest ancestor that has a role. Cfg'd out by default so the
    /// fragment is unchanged in the default build (AC-11.1).
    #[cfg(feature = "pdf-ua")]
    pub role: Option<UaRole>,
    /// Alternate text for a [`UaRole::Figure`] (`<img alt>`), written as `/Alt`.
    #[cfg(feature = "pdf-ua")]
    pub alt: Option<String>,
}

/// Cross-reference data a fragment carries when the `xref` feature is on: an
/// optional named destination it defines and an optional internal-link target it
/// activates. Boxed into one struct so the gated field is a single addition.
#[cfg(feature = "xref")]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XrefMeta {
    /// The `name` of a `<t:anchor name>` positioned at this fragment.
    pub anchor: Option<String>,
    /// The bare destination name from an `<a href="#name">` whose box this is.
    pub link_href: Option<String>,
}

impl Fragment {
    /// A fragment with default break metadata and no children.
    pub fn new(
        node_id: NodeId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        content: FragmentContent,
    ) -> Fragment {
        Fragment {
            node_id,
            x,
            y,
            width,
            height,
            content,
            break_meta: BreakMeta::default(),
            children: Vec::new(),
            z_index: None,
            is_positioned: false,
            #[cfg(feature = "xref")]
            xref: XrefMeta::default(),
            #[cfg(feature = "pdf-ua")]
            role: None,
            #[cfg(feature = "pdf-ua")]
            alt: None,
        }
    }

    /// The bottom edge of this fragment (`y + height`).
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// The effective stacking level used to order this fragment against its
    /// siblings when painting: the used `z-index` for a positioned box, else `0`
    /// (an in-flow box stacks at level 0, ignoring any stray `z-index`).
    pub fn paint_z(&self) -> i32 {
        if self.is_positioned {
            self.z_index.unwrap_or(0)
        } else {
            0
        }
    }

    /// This fragment's child indices in back-to-front paint order (CSS 2.2 §9.9,
    /// a pragmatic subset): a *stable* sort by `(paint_z, is_positioned)`, so
    /// within one stacking context negative-`z` boxes paint first, then in-flow
    /// non-positioned boxes, then `z:auto`/`0` positioned boxes, then positive-`z`
    /// — with source (DOM) order breaking ties. The children `Vec` itself keeps
    /// its layout (top-down galley) order so the paginator's flow walk is
    /// unaffected; only painters reorder via this view.
    pub fn paint_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.children.len()).collect();
        order.sort_by_key(|&i| {
            let c = &self.children[i];
            (c.paint_z(), c.is_positioned)
        });
        order
    }

    /// Shift this fragment and its whole subtree by `(dx, dy)`.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        for child in &mut self.children {
            child.translate(dx, dy);
        }
    }
}

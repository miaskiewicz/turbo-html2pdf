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
        }
    }

    /// The bottom edge of this fragment (`y + height`).
    pub fn bottom(&self) -> f32 {
        self.y + self.height
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

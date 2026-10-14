//! Fonts + inline/text layout (§4.4, §5.2): caller-supplied font faces, a
//! registry with weight/style selection and per-character fallback, and greedy
//! line layout with metrics and alignment.

#[cfg(feature = "bundled-fonts")]
mod bundled;
mod font;
mod inline;
mod platform_fallback;
mod registry;

pub use font::{FontFace, HtmlFontMetrics, ShapedGlyph};
pub use inline::{layout_text, Align, LineBox, TextStyle, WhiteSpace};
pub use registry::FontRegistry;

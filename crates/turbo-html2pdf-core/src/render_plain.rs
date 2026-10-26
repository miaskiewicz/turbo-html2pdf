//! [`Run`](super::Run)'s layout without the `pdf-ua`/`xref` features: no
//! semantics are recorded, so box ids need not be unique across flows.

use turbo_dom_layout::StyledNode;

use super::Run;
use crate::flow::Flow;
use crate::layout::fragment::Fragment;
use crate::layout::ImageCtx;
use crate::paginate::Page;
use crate::text::FontRegistry;

impl Run<'_> {
    /// Lay one styled flow out into a galley (no semantics in this build, so the
    /// box ids need not be unique across flows).
    pub(super) fn lay(
        &mut self,
        _flow: &Flow,
        styled: &[StyledNode],
        width: f32,
        fonts: &FontRegistry,
        images: &ImageCtx,
    ) -> Fragment {
        let options = crate::flow::layout_options(0);
        turbo_dom_layout::layout_with_options(styled, width, fonts, images, &options, self.diags)
    }

    /// No semantics in this build: the pages keep their empty table.
    pub(super) fn finish(self, _pages: &mut [Page]) {}
}

//! [`Run`](super::Run)'s layout with the `pdf-ua`/`xref` features: every flow gets
//! its own box-id range and its recorded box sources feed the page semantics.

use turbo_dom_layout::{LaidOut, StyledNode};

use super::Run;
use crate::flow::Flow;
use crate::layout::fragment::Fragment;
use crate::layout::ImageCtx;
use crate::paginate::Page;
use crate::text::FontRegistry;

impl Run<'_> {
    /// Lay one styled flow of `flow`'s document out into a galley with the next
    /// id range, recording its semantics.
    pub(super) fn lay(
        &mut self,
        flow: &Flow,
        styled: &[StyledNode],
        width: f32,
        fonts: &FontRegistry,
        images: &ImageCtx,
    ) -> Fragment {
        self.lay_recorded(flow, styled, width, fonts, images).galley
    }

    /// [`lay`](Self::lay) that also returns the boxes' sources.
    pub(super) fn lay_recorded(
        &mut self,
        flow: &Flow,
        styled: &[StyledNode],
        width: f32,
        fonts: &FontRegistry,
        images: &ImageCtx,
    ) -> LaidOut {
        let options = crate::flow::layout_options(self.next_id);
        let laid =
            turbo_dom_layout::layout_recorded(styled, width, fonts, images, &options, self.diags);
        self.next_id = laid.sources.next.0;
        self.derived.record(&flow.view(), &laid.sources);
        laid
    }

    /// Hand the recorded semantics to every page.
    pub(super) fn finish(self, pages: &mut [Page]) {
        let semantics = crate::semantics::Semantics::from_derived(self.derived);
        for page in pages {
            page.semantics = semantics.clone();
        }
    }
}

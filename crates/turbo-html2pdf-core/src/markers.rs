//! html2pdf's `t:` paged-media directives (§3) as turbo-dom-layout markers.
//!
//! The layout engine knows nothing about paged media: it lays a classified element
//! out as an opaque [`MarkerKind`] (an inline marker or a block box). This module
//! is the classifier html2pdf hands it — `t:footnote` → [`TKind::Footnote`] → a
//! marker, and back — so the fragmenter/emitter can recognize the markers in the
//! galley ([`FragmentContent::Marker`]).

use crate::layout::fragment::FragmentContent;
use turbo_dom_layout::MarkerKind;

/// A recognized `t:` paged-media directive element (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TKind {
    Footnote,
    FootnoteSeparator,
    RunningHeader,
    RunningFooter,
    PageMaster,
    Region,
    Variant,
    Page,
    Pages,
    UseMaster,
    Counter,
    Leader,
    Anchor,
    Endnote,
    Endnotes,
}

/// Every directive, in discriminant order (`ALL[k as usize] == k`).
const ALL: [TKind; 15] = [
    TKind::Footnote,
    TKind::FootnoteSeparator,
    TKind::RunningHeader,
    TKind::RunningFooter,
    TKind::PageMaster,
    TKind::Region,
    TKind::Variant,
    TKind::Page,
    TKind::Pages,
    TKind::UseMaster,
    TKind::Counter,
    TKind::Leader,
    TKind::Anchor,
    TKind::Endnote,
    TKind::Endnotes,
];

/// Map a `t:`-stripped local name to its directive kind, or `None` if unknown.
pub fn t_kind(name: &str) -> Option<TKind> {
    let kind = match name {
        "footnote" => TKind::Footnote,
        "footnote-separator" => TKind::FootnoteSeparator,
        "running-header" => TKind::RunningHeader,
        "running-footer" => TKind::RunningFooter,
        "page-master" => TKind::PageMaster,
        "region" => TKind::Region,
        "variant" => TKind::Variant,
        "page" => TKind::Page,
        "pages" => TKind::Pages,
        "use-master" => TKind::UseMaster,
        "counter" => TKind::Counter,
        "leader" => TKind::Leader,
        "anchor" => TKind::Anchor,
        "endnote" => TKind::Endnote,
        "endnotes" => TKind::Endnotes,
        _ => return None,
    };
    Some(kind)
}

impl TKind {
    /// The directive's `t:`-stripped element name (the inverse of [`t_kind`]).
    pub fn name(self) -> &'static str {
        match self {
            TKind::Footnote => "footnote",
            TKind::FootnoteSeparator => "footnote-separator",
            TKind::RunningHeader => "running-header",
            TKind::RunningFooter => "running-footer",
            TKind::PageMaster => "page-master",
            TKind::Region => "region",
            TKind::Variant => "variant",
            TKind::Page => "page",
            TKind::Pages => "pages",
            TKind::UseMaster => "use-master",
            TKind::Counter => "counter",
            TKind::Leader => "leader",
            TKind::Anchor => "anchor",
            TKind::Endnote => "endnote",
            TKind::Endnotes => "endnotes",
        }
    }

    /// Whether the directive sits inline within text flow (vs. a block box).
    pub fn is_inline(self) -> bool {
        matches!(
            self,
            TKind::Footnote
                | TKind::Page
                | TKind::Pages
                | TKind::Counter
                | TKind::Leader
                | TKind::Anchor
        )
    }

    /// The layout marker for this directive.
    pub fn marker(self) -> MarkerKind {
        MarkerKind {
            id: self as u16,
            inline: self.is_inline(),
        }
    }

    /// The directive a layout marker stands for (`None` for a foreign marker).
    pub fn from_marker(kind: MarkerKind) -> Option<TKind> {
        ALL.get(usize::from(kind.id))
            .copied()
            .filter(|t| t.marker() == kind)
    }

    /// The directive a fragment marks, if it is a directive marker fragment.
    pub fn of_fragment(content: &FragmentContent) -> Option<TKind> {
        match content {
            FragmentContent::Marker(kind) => TKind::from_marker(*kind),
            _ => None,
        }
    }
}

/// The directive an element's local name names (`t:footnote`, …), or `None`.
pub fn directive_of(local: &str) -> Option<TKind> {
    local.strip_prefix("t:").and_then(t_kind)
}

/// The marker classifier html2pdf hands the layout engine: `t:` directives become
/// markers; every other element (including an unknown `t:` name, which the
/// template parse rejects before layout) stays a plain element.
pub fn classify(local: &str) -> Option<MarkerKind> {
    directive_of(local).map(TKind::marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip_every_directive() {
        for (i, kind) in ALL.iter().enumerate() {
            assert_eq!(*kind as usize, i);
            assert_eq!(t_kind(kind.name()), Some(*kind));
            assert_eq!(TKind::from_marker(kind.marker()), Some(*kind));
        }
        let foreign = MarkerKind {
            id: TKind::Page as u16,
            inline: false,
        };
        assert_eq!(TKind::from_marker(foreign), None);
        assert_eq!(
            TKind::from_marker(MarkerKind {
                id: 99,
                inline: true
            }),
            None
        );
        assert_eq!(classify("t:anchor"), Some(TKind::Anchor.marker()));
        assert_eq!(classify("t:bogus"), None);
        assert_eq!(classify("div"), None);
        assert_eq!(
            TKind::of_fragment(&FragmentContent::Marker(TKind::Leader.marker())),
            Some(TKind::Leader)
        );
    }
}

use crate::{ComputedStyle, StyledElement, StyledNode, Tag};

fn is_contents(element: &StyledElement) -> bool {
    element.style.get("display").map(str::trim) == Some("contents")
}

/// Replaced HTML content has no independent CSS children to promote. CSS
/// Display's unusual-element rules treat these elements as display:none.
pub(crate) fn suppresses_subtree(element: &StyledElement) -> bool {
    is_contents(element)
        && matches!(&element.tag, Tag::Html(tag) if matches!(tag.as_str(),
        "br" | "wbr" | "meter" | "progress" | "canvas" | "embed" | "object" |
        "audio" | "iframe" | "img" | "video" | "frame" | "frameset" | "input" |
        "textarea" | "select"))
}

/// Flatten only the box tree. Original styled nodes, inherited text styles and
/// source ancestry stay intact, including across multiple unboxed ancestors.
pub(crate) fn visit<'a>(
    children: &'a [StyledNode],
    parent_style: &'a ComputedStyle,
    visitor: &mut impl FnMut(&'a StyledNode, &'a ComputedStyle),
) {
    for node in children {
        match node {
            StyledNode::Element(element) if suppresses_subtree(element) => {}
            StyledNode::Element(element) if is_contents(element) => {
                visit(&element.children, &element.style, visitor);
            }
            _ => visitor(node, parent_style),
        }
    }
}

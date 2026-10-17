use std::collections::HashMap;
use turbo_html2pdf_core::layout::inline::{
    layout_html_paragraph_in_with_sources, layout_paragraph_in_with_sources, InlineEdge, InlineRun,
    Piece,
};
use turbo_html2pdf_core::layout::value::{Rgba, VAlign};
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, GeometryKind, LayoutSnapshot, NativeTextOptions,
    SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::text::Align;
use turbo_html2pdf_core::{
    build_cascade, parse_html, Diagnostics, FontRegistry, ImageCtx, NoImages, Node, NodeId,
};

fn source_pieces(fonts: &FontRegistry, contributes: bool, line_height: Option<f32>) -> Vec<Piece> {
    let face = fonts.select(&["sans-serif"], 400, false).unwrap().clone();
    let edge = InlineEdge {
        node_id: NodeId(7),
        start: true,
        advance: 3.0,
        face: face.clone(),
        font_size: 32.0,
        line_height,
        baseline_shift: 0.0,
        contributes_metrics: contributes,
        valign: VAlign::Super,
    };
    let mut end = edge.clone();
    end.start = false;
    end.advance = 5.0;
    vec![
        Piece::Edge(edge),
        Piece::Run(InlineRun {
            node_id: NodeId(8),
            text: "abc".into(),
            face,
            families: vec!["sans-serif".into()],
            weight: 400,
            italic: false,
            font_size: 12.0,
            line_height: Some(12.0),
            letter_spacing: 0.0,
            color: Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
            valign: VAlign::Baseline,
            nowrap: false,
        }),
        Piece::Edge(end),
    ]
}

#[test]
fn public_source_paragraphs_keep_edge_advances_and_optional_font_metrics() {
    let fonts = FontRegistry::new();
    let region = |_: f32| (0.0, 400.0);
    for html in [false, true] {
        let capture = |pieces: &[Piece]| {
            let layout = if html {
                layout_html_paragraph_in_with_sources
            } else {
                layout_paragraph_in_with_sources
            };
            layout(
                pieces,
                &fonts,
                Align::Left,
                &mut Diagnostics::default(),
                &region,
                12.0,
            )
        };
        let disabled = capture(&source_pieces(&fonts, false, Some(44.0)));
        let contributing = capture(&source_pieces(&fonts, true, Some(44.0)));
        let natural = capture(&source_pieces(&fonts, true, None));
        let pieces = source_pieces(&fonts, false, None);
        let plain = capture(&pieces[1..2]);
        assert_eq!(disabled.layout.lines.len(), 1);
        assert_eq!(disabled.layout.lines[0].inline_edges.len(), 2);
        assert_eq!(disabled.layout.width, plain.layout.width + 8.0);
        assert_eq!(disabled.layout.width, contributing.layout.width);
        assert!(contributing.layout.height >= 44.0);
        assert!(contributing.layout.height > disabled.layout.height);
        assert!(natural.layout.height > disabled.layout.height);
        assert!(natural.layout.lines[0].baseline > disabled.layout.lines[0].baseline);
        let original_offsets =
            |clusters: &[turbo_html2pdf_core::layout::inline::GlyphSourceCluster]| {
                clusters
                    .iter()
                    .map(|cluster| (cluster.utf16_start, cluster.utf16_end, cluster.advance))
                    .collect::<Vec<_>>()
            };
        assert_eq!(
            original_offsets(&disabled.clusters[0][0]),
            original_offsets(&plain.clusters[0][0])
        );
        assert_eq!(disabled.layout.lines[0].runs[0].node_id, NodeId(8));
    }
}

fn identities(
    nodes: &[Node],
    path: &mut Vec<usize>,
    out: &mut Vec<SourceIdentity>,
    names: &mut HashMap<String, SourceId>,
) {
    for (index, node) in nodes.iter().enumerate() {
        path.push(index);
        let id = SourceId(out.len() as u64 + 1);
        out.push(SourceIdentity {
            id,
            path: path.clone(),
        });
        if let Node::Element(element) = node {
            for attribute in &element.attrs {
                if attribute.name == "id" {
                    names.insert(attribute.value.clone(), id);
                }
            }
            identities(&element.children, path, out, names);
        }
        path.pop();
    }
}

fn snapshot(html: &str) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    snapshot_nodes(parse_html(html).unwrap())
}
fn snapshot_nodes(nodes: Vec<Node>) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    let mut ids = Vec::new();
    let mut names = HashMap::new();
    identities(&nodes, &mut Vec::new(), &mut ids, &mut names);
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let cascade = build_cascade(
        "div{font-family:sans-serif;font-size:16px;line-height:24px;width:160px}",
        "",
        Default::default(),
    );
    let snapshot = snapshot_html_live_tree_with_native_text(
        &nodes,
        &ids,
        &cascade,
        &NativeTextOptions {
            original_text: &[],
            svg_viewports: &[],
            layout_scale: 2.0,
        },
        &SnapshotInputs {
            width: 200.0,
            height: 200.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    )
    .unwrap();
    (snapshot, names)
}

fn first_box(snapshot: &LayoutSnapshot, id: SourceId) -> [f32; 4] {
    let fragment = snapshot
        .source(id)
        .unwrap()
        .fragment_indices
        .iter()
        .map(|index| &snapshot.fragments()[*index])
        .find(|fragment| fragment.kind == GeometryKind::BorderBox)
        .unwrap();
    let rect = fragment.rect;
    [rect.x, rect.y, rect.width, rect.height]
}

#[test]
fn collapsed_empty_inline_sources_keep_zero_boxes_and_relative_position() {
    let source = "<div><span id=empty style='{};padding-top:3px'>  </span></div>";
    let (plain, names) = snapshot(&source.replace("{}", "position:relative"));
    let (shifted, shifted_names) =
        snapshot(&source.replace("{}", "position:relative;left:10px;top:7px"));
    let before = first_box(&plain, names["empty"]);
    assert_eq!(before[2..], [0.0, 0.0]);
    assert_eq!(
        first_box(&shifted, shifted_names["empty"]),
        [before[0] + 10.0, before[1] + 7.0, 0.0, 0.0]
    );
    let text = plain
        .text_sources()
        .iter()
        .find(|text| text.text == "  ")
        .unwrap();
    assert!(plain
        .text_range_boxes(text.source_id, 0, 2)
        .unwrap()
        .is_empty());
}

#[test]
fn hidden_subtrees_and_contents_wrappers_preserve_visible_source_geometry() {
    let (snapshot, names) = snapshot("<div><span id=hidden style='display:none'><b style='direction:rtl;position:absolute'>hidden</b></span><span id=contents style='display:contents'><span id=visible style='padding:2px;border:1px solid'>visible</span></span></div>");
    assert!(
        snapshot
            .source(names["hidden"])
            .unwrap()
            .excluded_from_layout
    );
    assert!(snapshot
        .source(names["contents"])
        .unwrap()
        .fragment_indices
        .is_empty());
    assert!(first_box(&snapshot, names["visible"])[2] > 6.0);
}

#[test]
fn zero_advance_cells_do_not_add_empty_rectangles_to_a_nonempty_selection() {
    let (snapshot, _) = snapshot("<div>a\u{200b}b</div>");
    let text = snapshot
        .text_sources()
        .iter()
        .find(|text| text.text == "a\u{200b}b")
        .unwrap();
    let cells = snapshot.text_cluster_boxes_for_source(text.source_id);
    assert!(cells.iter().any(|cell| cell.rect.width == 0.0));
    let full = snapshot.text_range_boxes(text.source_id, 0, 3).unwrap();
    assert!(!full.is_empty());
    assert!(full.iter().all(|rect| rect.rect.width > 0.0));
    let zero = snapshot.text_range_boxes(text.source_id, 1, 2).unwrap();
    assert!(!zero.is_empty());
    assert!(zero.iter().all(|rect| rect.rect.width == 0.0));
}

#[test]
fn empty_inline_inside_visible_text_keeps_its_insertion_box_without_advance() {
    let (plain, plain_names) = snapshot("<div id=box>ab</div>");
    let (marked, names) = snapshot("<div id=box><span>a<span id=empty></span>b</span></div>");
    let empty = first_box(&marked, names["empty"]);
    assert_eq!(empty[2], 0.0);
    assert!(empty[3] > 0.0);
    assert_eq!(
        first_box(&plain, plain_names["box"]),
        first_box(&marked, names["box"])
    );
    let before = marked
        .text_sources()
        .iter()
        .find(|text| text.text == "a")
        .unwrap();
    let after = marked
        .text_sources()
        .iter()
        .find(|text| text.text == "b")
        .unwrap();
    let left = marked.text_cluster_boxes_for_source(before.source_id);
    let right = marked.text_cluster_boxes_for_source(after.source_id);
    assert_eq!(empty[0], left[0].rect.x + left[0].rect.width);
    assert_eq!(empty[0], right[0].rect.x);
}

#[test]
fn adjacent_unbroken_source_runs_and_multiline_inline_metrics_retain_their_owners() {
    let fonts = FontRegistry::new();
    let pieces = source_pieces(&fonts, true, Some(44.0));
    let Piece::Run(first) = &pieces[1] else {
        unreachable!()
    };
    let mut second = first.clone();
    second.node_id = NodeId(9);
    second.text = "def".into();
    second.color.r = 64;
    let mut third = first.clone();
    third.node_id = NodeId(10);
    third.text = "ghi".into();
    let region = |_: f32| (0.0, 400.0);
    for html in [false, true] {
        let layout = if html {
            layout_html_paragraph_in_with_sources
        } else {
            layout_paragraph_in_with_sources
        };
        let joined = layout(
            &[
                pieces[0].clone(),
                pieces[1].clone(),
                Piece::Run(second.clone()),
                pieces[2].clone(),
            ],
            &fonts,
            Align::Left,
            &mut Diagnostics::default(),
            &region,
            12.0,
        );
        assert_eq!(joined.layout.lines.len(), 1);
        assert_eq!(
            joined.layout.lines[0]
                .runs
                .iter()
                .map(|run| run.node_id)
                .collect::<Vec<_>>(),
            [NodeId(8), NodeId(9)]
        );
        assert_eq!(joined.clusters[0].len(), 2);
        assert_eq!(joined.clusters[0][0].first().unwrap().utf16_start, 0);
        assert_eq!(joined.clusters[0][1].last().unwrap().utf16_end, 3);
        let wrapped = layout(
            &[
                pieces[0].clone(),
                pieces[1].clone(),
                Piece::Break,
                Piece::Run(second.clone()),
                Piece::Break,
                Piece::Run(third.clone()),
                pieces[2].clone(),
            ],
            &fonts,
            Align::Left,
            &mut Diagnostics::default(),
            &region,
            12.0,
        );
        assert_eq!(wrapped.layout.lines.len(), 3);
        assert!(wrapped.layout.lines[1].inline_edges.is_empty());
        assert!(wrapped.layout.lines.iter().all(|line| line.height >= 44.0));
        assert_eq!(
            wrapped.layout.lines[1].baseline,
            wrapped.layout.lines[0].baseline
        );
        assert_eq!(wrapped.layout.lines[1].runs[0].node_id, NodeId(9));
    }
}

#[test]
fn an_unmatched_public_closing_edge_keeps_its_advance_without_inventing_glyphs() {
    let fonts = FontRegistry::new();
    let pieces = source_pieces(&fonts, false, None);
    let paragraph = layout_paragraph_in_with_sources(
        &pieces[2..],
        &fonts,
        Align::Left,
        &mut Diagnostics::default(),
        &|_| (0.0, 200.0),
        12.0,
    );
    assert_eq!(paragraph.layout.lines.len(), 1);
    assert_eq!(paragraph.layout.width, 5.0);
    assert_eq!(paragraph.layout.lines[0].inline_edges.len(), 1);
    assert!(!paragraph.layout.lines[0].inline_edges[0].edge.start);
    assert!(paragraph.layout.lines[0].runs.is_empty());
    assert!(paragraph.clusters[0].is_empty());
}

#[test]
fn native_fixed_replacements_resolve_percentage_height_against_the_viewport() {
    let (snapshot, names) = snapshot("<div style='height:40px'><iframe id=frame style='display:inline;position:fixed;left:0;top:0;width:10px;height:50%;border:0'></iframe></div>");
    assert_eq!(
        first_box(&snapshot, names["frame"]),
        [0.0, 0.0, 10.0, 100.0]
    );
}

#[test]
fn own_atomic_super_and_sub_alignment_shift_once_without_growing_a_dominant_strut() {
    let capture = |align| {
        snapshot(&format!("<div id=box><iframe id=frame style='display:inline;width:20px;height:5px;border:0;vertical-align:{align}'></iframe></div>"))
    };
    let (normal, names) = capture("baseline");
    let baseline = first_box(&normal, names["frame"]);
    for (align, delta) in [("super", -5.828125), ("sub", 3.6953125)] {
        let (shifted, shifted_names) = capture(align);
        let actual = first_box(&shifted, shifted_names["frame"]);
        assert_eq!(actual, [baseline[0], baseline[1] + delta, 20.0, 5.0]);
        assert_eq!(
            first_box(&shifted, shifted_names["box"]),
            first_box(&normal, names["box"])
        );
    }
}

#[test]
fn preserved_whitespace_and_inline_directives_keep_real_source_provenance() {
    let (pre, _) = snapshot("<div><span style='white-space:pre'>  </span></div>");
    let text = pre
        .text_sources()
        .iter()
        .find(|source| source.text == "  ")
        .unwrap();
    let selected = pre.text_range_boxes(text.source_id, 0, 2).unwrap();
    assert!(!selected.is_empty());
    assert!(selected.iter().any(|part| part.rect.width > 0.0));
    let mut nodes = parse_html("<div>a<span id=marker></span>b</div>").unwrap();
    let Node::Element(parent) = &mut nodes[0] else {
        unreachable!()
    };
    let Node::Element(marker) = &mut parent.children[1] else {
        unreachable!()
    };
    marker.tag = turbo_html2pdf_core::Tag::Directive(turbo_html2pdf_core::TKind::Page);
    let (snapshot, names) = snapshot_nodes(nodes);
    let source = snapshot.source(names["marker"]).unwrap();
    assert!(!source.excluded_from_layout);
    assert_eq!(source.fragment_indices.len(), 1);
    let fragment = &snapshot.fragments()[source.fragment_indices[0]];
    assert_eq!(fragment.kind, GeometryKind::Directive);
    assert_eq!([fragment.rect.width, fragment.rect.height], [0.0, 0.0]);
    assert_eq!(snapshot.text_sources().len(), 2);
    assert!(snapshot.text_sources().iter().all(|text| !snapshot
        .text_cluster_boxes_for_source(text.source_id)
        .is_empty()));
}

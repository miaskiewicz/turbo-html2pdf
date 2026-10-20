use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, GeometryKind, LayoutSnapshot, NativeTextOptions,
    SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, parse_html, Diagnostics, FontRegistry, ImageCtx, NoImages, Node,
};

const CASES: [(&str, &str); 6] = [
    ("exactOneBlock", "<a><h3>Old result</h3></a>"),
    ("mixedContinuation", "<a>Before<h3>Old result</h3>After</a>"),
    ("nestedInline", "<a><span id='nested'>Before<h3>Old result</h3>After</span></a>"),
    ("nestedOuterInline", "<span id='nested'>Outer<a>Before<h3>Old result</h3>After</a>Tail</span>"),
    ("multipleBlocks", "<a>Before<h3>First</h3>Between<div>Second</div>After</a><section id='sibling'>Sibling</section>"),
    ("decorated", "<a style='border:2px solid;padding:3px'>Before<h3>Old result</h3>After</a>"),
];
fn identities(
    nodes: &[Node],
    path: &mut Vec<usize>,
    ids: &mut Vec<SourceIdentity>,
    names: &mut HashMap<String, SourceId>,
) {
    for (index, node) in nodes.iter().enumerate() {
        path.push(index);
        let id = SourceId(ids.len() as u64 + 1);
        ids.push(SourceIdentity {
            id,
            path: path.clone(),
        });
        if let Node::Element(element) = node {
            record_names(element, id, names);
            identities(&element.children, path, ids, names);
        }
        path.pop();
    }
}
fn record_names(
    element: &turbo_html2pdf_core::Element,
    id: SourceId,
    names: &mut HashMap<String, SourceId>,
) {
    if let turbo_html2pdf_core::Tag::Html(tag) = &element.tag {
        names.entry(tag.clone()).or_insert(id);
    }
    for attr in &element.attrs {
        if attr.name == "id" {
            names.insert(attr.value.clone(), id);
        }
    }
}
fn fonts() -> &'static FontRegistry {
    static FONTS: OnceLock<FontRegistry> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts = FontRegistry::new();
        fonts.load_system_fonts_for_html();
        fonts
    })
}
type Captured = (LayoutSnapshot, HashMap<String, SourceId>);
fn capture(markup: &str) -> Captured {
    capture_result(markup).unwrap()
}
fn capture_result(
    markup: &str,
) -> Result<Captured, turbo_html2pdf_core::layout_snapshot::SvgSnapshotError> {
    capture_with_partial_identities(markup, false)
}
fn capture_with_partial_identities(
    markup: &str,
    omit_inline: bool,
) -> Result<Captured, turbo_html2pdf_core::layout_snapshot::SvgSnapshotError> {
    let html = format!("<div id=rso>{markup}</div><button id=dismiss>Reject</button><input name=q type=hidden value=hidden><form><textarea name=q></textarea><button>Search</button></form>");
    let element = |tag: &str, children| {
        Node::Element(turbo_html2pdf_core::Element {
            tag: turbo_html2pdf_core::Tag::Html(tag.into()),
            attrs: Vec::new(),
            children,
        })
    };
    // parse_html intentionally returns PDF body flow; the live caller owns real document roots.
    let nodes = vec![element(
        "html",
        vec![
            element("head", Vec::new()),
            element("body", parse_html(&html).unwrap()),
        ],
    )];
    let mut ids = Vec::new();
    let mut names = HashMap::new();
    identities(&nodes, &mut Vec::new(), &mut ids, &mut names);
    if omit_inline {
        ids.retain(|identity| identity.id != names["a"]);
    }
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let snapshot = snapshot_html_live_tree_with_native_text(&nodes, &ids,
        &build_cascade("body{margin:8px;font-family:serif}head,style,script,title,meta,link,base,noscript{display:none}button{font-family:Arial;font-size:13.3333px;line-height:normal;padding:1px 6px;border:2px solid;box-sizing:border-box}", "", Default::default()),
        &NativeTextOptions { original_text: &[], svg_viewports: &[], layout_scale: 2.0 },
        &SnapshotInputs { width:1280.0, height:800.0, fonts:fonts(), images:&images },
        &mut Diagnostics::default())?;
    Ok((snapshot, names))
}
fn rects(snapshot: &LayoutSnapshot, source: SourceId) -> Vec<[f64; 4]> {
    snapshot
        .source(source)
        .unwrap()
        .fragment_indices
        .iter()
        .map(|index| &snapshot.fragments()[*index])
        .filter(|fragment| fragment.kind == GeometryKind::BorderBox)
        .map(|fragment| {
            let r = fragment.rect;
            [r.x as f64, r.y as f64, r.width as f64, r.height as f64]
        })
        .collect()
}
#[test]
fn saved_chrome_probe_provenance_matches_reusable_source() {
    let chrome: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/block-inline-chrome155.json")).unwrap();
    assert_eq!(
        chrome["provenance"]["repositoryProbeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/block-inline-probe.js"))
        )
    );
    assert_eq!(chrome["provenance"]["browser"], "Chrome/155.0.8059.85");
    assert_eq!(chrome["provenance"]["v8"], "15.5.35.25");
}
#[cfg(target_os = "macos")]
#[test]
fn block_in_inline_fragments_and_neighbor_geometry_match_chrome() {
    let chrome: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/block-inline-chrome155.json")).unwrap();
    for (name, markup) in CASES {
        let (snapshot, names) = capture(markup);
        for (label, id) in [
            ("parent", "rso"),
            ("link", "a"),
            ("heading", "h3"),
            ("nested", "nested"),
            ("sibling", "sibling"),
            ("dismiss", "dismiss"),
        ] {
            if let Some(id) = names.get(id) {
                let expected: Vec<[f64; 4]> = serde_json::from_value(
                    chrome["variants"][name]["rows"][label]["rects"].clone(),
                )
                .unwrap();
                assert_eq!(rects(&snapshot, *id), expected, "{name} {label}");
            }
        }
        let heading = snapshot.source(names["h3"]).unwrap();
        let actual = &snapshot.fragments()[heading.fragment_indices[0]];
        assert!(actual.source_ancestors.contains(&names["a"]));
        assert_eq!(actual.source_id, Some(names["h3"]));
        assert!(snapshot
            .source(names["a"])
            .unwrap()
            .fragment_indices
            .contains(&actual.index));
    }
}
#[test]
fn fragment_ownership_preserves_ids_and_internal_href_does_not_atomize_native_blocks() {
    let (plain, names) = capture("<a>Before<h3>Old result</h3>After</a>");
    let (href, href_names) = capture("<a href='#target'>Before<h3>Old result</h3>After</a>");
    assert_eq!(rects(&plain, names["a"]), rects(&href, href_names["a"]));
    assert_eq!(rects(&plain, names["h3"]), rects(&href, href_names["h3"]));
    assert_eq!(plain.sources().len(), href.sources().len());
    assert_eq!(plain.text_sources(), href.text_sources());
}

#[test]
fn real_empty_blocks_keep_their_height_borders_and_flow_advance() {
    let (plain, names) = capture("<a><h3>Old result</h3></a>");
    let (spaced, spaced_names) = capture("<div id=spacer style='height:100px;border:1px solid;margin:0'> </div><a><h3>Old result</h3></a>");
    let spacer = rects(&spaced, spaced_names["spacer"])[0];
    assert_eq!(spacer[3], 102.0);
    assert!(rects(&spaced, spaced_names["h3"])[0][1] >= spacer[1] + spacer[3]);
    assert!(
        rects(&spaced, spaced_names["dismiss"])[0][1]
            > rects(&plain, names["dismiss"])[0][1] + 100.0
    );
}

#[test]
fn all_static_split_variants_preserve_original_source_ownership() {
    for (name, markup) in CASES {
        let (snapshot, names) = capture(markup);
        let link = snapshot.source(names["a"]).unwrap();
        let heading = snapshot.source(names["h3"]).unwrap();
        let block = &snapshot.fragments()[heading.fragment_indices[0]];
        assert_eq!(block.source_id, Some(names["h3"]), "{name}");
        assert!(block.source_ancestors.contains(&names["a"]), "{name}");
        assert!(link.fragment_indices.contains(&block.index), "{name}");
        assert!(
            rects(&snapshot, names["dismiss"])[0][1]
                >= block.rect.y as f64 + block.rect.height as f64,
            "{name}"
        );
    }
}

#[test]
fn unsupported_displaced_splits_reject_but_boxless_text_does_not() {
    for markup in [
        "<a style='position:relative;left:5px'><h3>block</h3></a>",
        "<a style='float:left'><h3>block</h3></a>",
        "<a><div style='position:absolute'>block</div></a>",
        "<a><div style='float:left'>block</div></a>",
    ] {
        match capture_result(markup) {
            Err(error) => assert_eq!(error, turbo_html2pdf_core::layout_snapshot::SvgSnapshotError::from(turbo_html2pdf_core::layout_snapshot::SnapshotError::UnsupportedInlineBlockSplit)),
            Ok(_) => panic!("unsupported displaced split accepted: {markup}"),
        }
    }
    let (plain, names) = capture("<a style='position:relative;left:5px'>Before</a>");
    let (contents, contents_names) = capture(
        "<a style='position:relative;left:5px'><span style='display:contents'>Before</span></a>",
    );
    assert_eq!(
        rects(&plain, names["a"]),
        rects(&contents, contents_names["a"])
    );
}

#[test]
fn escaping_percentage_bottom_margin_uses_its_actual_containing_width() {
    for padding in ["", "padding:0 15px;"] {
        let markup = |margin| {
            format!("<div style='width:100px;{padding}'><a><h3 style='margin:0 0 {margin}'>Old result</h3></a></div>")
        };
        let (percentage, ids) = capture(&markup("10%"));
        let (pixels, pixel_ids) = capture(&markup("10px"));
        assert_eq!(
            rects(&percentage, ids["dismiss"]),
            rects(&pixels, pixel_ids["dismiss"])
        );
        assert_eq!(
            rects(&percentage, ids["h3"]),
            rects(&pixels, pixel_ids["h3"])
        );
    }
}

#[test]
fn partial_source_identities_keep_the_real_block_without_inventing_inline_owners() {
    let markup = "<a>Before<h3>Old result</h3>After</a>";
    let (complete, complete_ids) = capture(markup);
    let (partial, ids) = capture_with_partial_identities(markup, true).unwrap();
    assert!(partial.source(ids["a"]).is_none());
    assert_eq!(partial.sources().len() + 1, complete.sources().len());
    assert_eq!(
        rects(&partial, ids["h3"]),
        rects(&complete, complete_ids["h3"])
    );
    assert_eq!(
        rects(&partial, ids["dismiss"]),
        rects(&complete, complete_ids["dismiss"])
    );
    let heading = partial.source(ids["h3"]).unwrap();
    let block = &partial.fragments()[heading.fragment_indices[0]];
    assert_eq!(block.source_id, Some(ids["h3"]));
    assert!(!block.source_ancestors.contains(&ids["a"]));
}

#[test]
fn hidden_blocks_do_not_displace_or_reject_a_relative_inline() {
    let (plain, names) = capture("<a style='position:relative;left:5px'>Before</a>");
    let (hidden, hidden_names) = capture("<a style='position:relative;left:5px'>Before<span style='display:none'><h3>hidden</h3></span></a>");
    assert_eq!(rects(&hidden, hidden_names["a"]), rects(&plain, names["a"]));
    assert_eq!(
        rects(&hidden, hidden_names["dismiss"]),
        rects(&plain, names["dismiss"])
    );
}

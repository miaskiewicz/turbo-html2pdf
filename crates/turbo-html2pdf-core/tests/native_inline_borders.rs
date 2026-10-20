use std::collections::HashMap;
use std::sync::OnceLock;
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, GeometryKind, LayoutSnapshot, NativeTextOptions,
    SnapshotInputs, SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, parse_html, Diagnostics, FontRegistry, ImageCtx, NoImages, Node,
};

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
            record_name(element, id, names);
            identities(&element.children, path, out, names);
        }
        path.pop();
    }
}
fn record_name(
    element: &turbo_html2pdf_core::Element,
    id: SourceId,
    names: &mut HashMap<String, SourceId>,
) {
    for attribute in &element.attrs {
        if attribute.name == "id" {
            names.insert(attribute.value.clone(), id);
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
fn capture(html: &str) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    capture_result(html).unwrap()
}
fn capture_result(
    html: &str,
) -> Result<
    (LayoutSnapshot, HashMap<String, SourceId>),
    turbo_html2pdf_core::layout_snapshot::SvgSnapshotError,
> {
    let nodes = parse_html(html).unwrap();
    let mut ids = Vec::new();
    let mut names = HashMap::new();
    identities(&nodes, &mut Vec::new(), &mut ids, &mut names);
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let cascade=build_cascade("#box{font-family:monospace;font-size:16px;line-height:24px;width:75px}span{vertical-align:baseline}","",Default::default());
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
            fonts: fonts(),
            images: &images,
        },
        &mut Diagnostics::default(),
    )?;
    Ok((snapshot, names))
}
fn rects(snapshot: &LayoutSnapshot, id: SourceId) -> Vec<[f64; 4]> {
    snapshot
        .source(id)
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
fn ordinary_inline_and_replaced_source_fragments_remain_distinct() {
    let (snapshot,names)=capture("<div id=box><span id=empty style='padding:3px 5px;border:2px solid;margin:0 7px'></span><img id=image style='display:inline;width:20px;height:10px;padding:3px;border:2px solid'></div>");
    let empty = snapshot.source(names["empty"]).unwrap();
    let image = snapshot.source(names["image"]).unwrap();
    assert!(
        snapshot.fragments()[empty.fragment_indices[0]]
            .used_box
            .unwrap()
            .inline_fragment
    );
    assert!(
        !snapshot.fragments()[image.fragment_indices[0]]
            .used_box
            .unwrap()
            .inline_fragment
    );
    assert_eq!(rects(&snapshot, names["empty"])[0][2], 14.0);
    assert_eq!(rects(&snapshot, names["image"])[0][2..], [30.0, 20.0]);
}

#[test]
fn transparent_ancestors_reference_child_regions_without_repainting_child_alpha_or_borders() {
    use turbo_html2pdf_core::FragmentContent;
    let (snapshot,names)=capture("<div id=box><span id=outer><span id=inner style='background:rgba(255,0,0,.5);border:1px solid;padding:2px'>abc</span></span></div>");
    assert_eq!(
        rects(&snapshot, names["outer"]),
        rects(&snapshot, names["inner"])
    );
    fn paints(fragment: &turbo_html2pdf_core::Fragment) -> usize {
        let own = usize::from(matches!(
            &fragment.content,
            FragmentContent::Box {
                background: Some(_),
                ..
            }
        ));
        own + fragment.children.iter().map(paints).sum::<usize>()
    }
    assert_eq!(paints(snapshot.galley()), 1);
}

#[cfg(target_os = "macos")]
#[test]
fn authored_native_inline_fragments_match_independent_chrome_boundaries_fonts_and_slices() {
    use sha2::{Digest, Sha256};
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/inline-borders/cases.json")).unwrap();
    let control: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/inline-borders/chrome155.json")).unwrap();
    assert_eq!(control["browser"], "Chrome/155.0.8059.80");
    assert_eq!(control["errors"], serde_json::json!([]));
    assert_eq!(
        control["probeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/inline-borders/probe.js"))
        )
    );
    for case in cases.as_array().unwrap() {
        compare_case(case, &control);
    }
}
#[test]
fn authored_inline_graphs_retain_actual_source_roles_on_every_platform() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/inline-borders/cases.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let (snapshot, names) = case_snapshot(case);
        for (name, id) in names {
            check_source_role(&snapshot, id, &name, case);
        }
    }
}
fn case_snapshot(case: &serde_json::Value) -> (LayoutSnapshot, HashMap<String, SourceId>) {
    let html = format!(
        "<div id=box style='{}'>{}</div>",
        case["boxStyle"].as_str().unwrap(),
        case["html"].as_str().unwrap()
    );
    capture_result(&html).unwrap_or_else(|error| panic!("{}: {error}", case["name"]))
}
fn check_source_role(
    snapshot: &LayoutSnapshot,
    id: SourceId,
    name: &str,
    case: &serde_json::Value,
) {
    let atomic = expected_atomic(name, case);
    let source = snapshot.source(id).unwrap();
    assert_eq!(source.id, id);
    assert!(
        !source.fragment_indices.is_empty(),
        "{} {name}",
        case["name"]
    );
    for index in &source.fragment_indices {
        let fragment = &snapshot.fragments()[*index];
        if fragment.kind == GeometryKind::BorderBox {
            assert_eq!(
                fragment.used_box.unwrap().inline_fragment,
                !atomic,
                "{} {name}",
                case["name"]
            );
            let rect = fragment.rect;
            assert!([rect.x, rect.y, rect.width, rect.height]
                .iter()
                .all(|value| value.is_finite()));
            assert!(rect.width >= 0.0 && rect.height >= 0.0);
        }
    }
}
fn expected_atomic(name: &str, case: &serde_json::Value) -> bool {
    match name {
        "box" => true,
        "outer" => case["name"] == "replaced-image",
        "inner" => case["html"].as_str().unwrap().contains("display:inline"),
        _ => false,
    }
}
#[test]
fn relative_inline_offsets_move_descendants_without_changing_following_flow() {
    let template = "<div id=box><span id=outer style='{};padding:2px;border:1px solid'>a<span id=inner>b</span>c</span><span id=after>z</span></div>";
    let (plain, plain_names) = capture(&template.replace("{}", "position:relative"));
    let (shifted, shifted_names) =
        capture(&template.replace("{}", "position:relative;left:10px;top:6px"));
    for name in ["outer", "inner"] {
        let before = rects(&plain, plain_names[name]);
        let after = rects(&shifted, shifted_names[name]);
        assert_eq!(before.len(), after.len());
        for (before, after) in before.into_iter().zip(after) {
            assert_eq!(
                after,
                [before[0] + 10.0, before[1] + 6.0, before[2], before[3]]
            );
        }
    }
    assert_eq!(
        rects(&plain, plain_names["after"]),
        rects(&shifted, shifted_names["after"])
    );
}
#[cfg(target_os = "macos")]
fn compare_case(case: &serde_json::Value, control: &serde_json::Value) {
    let (snapshot, names) = case_snapshot(case);
    let expected = control["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == case["name"])
        .unwrap();
    for name in ["outer", "inner", "after"] {
        if let Some(id) = names.get(name) {
            let expected: Vec<[f64; 4]> =
                serde_json::from_value(expected[name]["rects"].clone()).unwrap();
            assert_eq!(rects(&snapshot, *id), expected, "{} {name}", case["name"]);
        }
    }
    let expected: [f64; 4] = serde_json::from_value(expected["box"].clone()).unwrap();
    assert_eq!(
        rects(&snapshot, names["box"])[0],
        expected,
        "{} line strut",
        case["name"]
    );
}

#[test]
fn unsupported_ordinary_inline_contexts_reject_without_inventing_partial_boxes() {
    use turbo_html2pdf_core::layout_snapshot::SnapshotError;
    let cases = [
        (
            "<span style='position:relative'><div>block</div></span>",
            SnapshotError::UnsupportedInlineBlockSplit,
        ),
        (
            "<span style='vertical-align:middle'>middle</span>",
            SnapshotError::UnsupportedInlineAlignment,
        ),
        (
            "<span style='direction:rtl'>rtl</span>",
            SnapshotError::UnsupportedInlineWriting,
        ),
        (
            "<span style='writing-mode:vertical-rl'>vertical</span>",
            SnapshotError::UnsupportedInlineWriting,
        ),
        (
            "<span style='position:absolute'>absolute</span>",
            SnapshotError::UnsupportedInlinePosition,
        ),
    ];
    for (html, expected) in cases {
        match capture_result(html) {
            Err(error) => assert_eq!(
                error,
                turbo_html2pdf_core::layout_snapshot::SvgSnapshotError::from(expected)
            ),
            Ok(_) => panic!("unsupported inline accepted: {html}"),
        }
    }
}

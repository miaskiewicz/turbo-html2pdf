use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, NativeTextOptions, SnapshotInputs, SourceId,
    SourceIdentity,
};
use turbo_html2pdf_core::{
    Attr, Diagnostics, Element, FontFace, FontRegistry, ImageCtx, Node, Tag,
};

fn nodes(text: &str, family: &str) -> Vec<Node> {
    vec![Node::Element(Element {
        tag: Tag::Html("div".into()),
        attrs: vec![Attr {
            name: "style".into(),
            value: format!("font-family:{family};font-size:64px;line-height:80px;width:400px"),
        }],
        children: vec![Node::Text(text.into())],
    })]
}
fn snapshot(
    units: &[u16],
    fonts: &FontRegistry,
    family: &str,
) -> Result<
    turbo_html2pdf_core::layout_snapshot::LayoutSnapshot,
    turbo_html2pdf_core::layout_snapshot::SvgSnapshotError,
> {
    let nodes = nodes(&String::from_utf16_lossy(units), family);
    let originals = [turbo_html2pdf_core::layout_snapshot::SourceTextUnits {
        source_id: SourceId(7),
        units,
    }];
    snapshot_html_live_tree_with_native_text(
        &nodes,
        &[SourceIdentity {
            id: SourceId(7),
            path: vec![0, 0],
        }],
        &turbo_html2pdf_core::build_cascade("", "", Default::default()),
        &NativeTextOptions {
            original_text: &originals,
            svg_viewports: &[],
            layout_scale: 2.0,
        },
        &SnapshotInputs {
            width: 800.0,
            height: 600.0,
            fonts,
            images: &ImageCtx {
                resolver: &turbo_html2pdf_core::NoImages,
                body_height: None,
            },
        },
        &mut Diagnostics::default(),
    )
}
#[test]
fn public_original_units_retain_native_source_identity_and_validation() {
    let mut fonts = FontRegistry::new();
    fonts.add(
        FontFace::from_bytes(
            include_bytes!("../assets/fonts/Go-Regular.ttf").to_vec(),
            "Go",
            400,
            false,
        )
        .unwrap(),
    );
    let units = [0x61, 0xd800, 0x62, 0xdcff, 0x63];
    let captured = snapshot(&units, &fonts, "Go").unwrap();
    assert_eq!(
        captured.text_original_units(SourceId(7)),
        Some(units.as_slice())
    );
    assert_eq!(captured.text_original_units(SourceId(99)), None);
    assert!(!captured
        .text_range_boxes(SourceId(7), 0, 5)
        .unwrap()
        .is_empty());
    assert!(captured.text_range_boxes(SourceId(7), 0, 6).is_err());
    assert!(captured.native_hit_transform(0).is_some());
    assert!(captured.native_hit_transform(usize::MAX).is_none());
    let missing = snapshot(&[0xdbff, 0xdfff], &fonts, "Go").unwrap();
    assert_eq!(
        missing.text_original_units(SourceId(7)),
        Some([0xdbff, 0xdfff].as_slice())
    );
    let nodes = nodes("a", "Go");
    let bad = [turbo_html2pdf_core::layout_snapshot::SourceTextUnits {
        source_id: SourceId(7),
        units: &[0x62],
    }];
    let options = NativeTextOptions {
        original_text: &bad,
        svg_viewports: &[],
        layout_scale: 2.0,
    };
    assert!(snapshot_html_live_tree_with_native_text(
        &nodes,
        &[SourceIdentity {
            id: SourceId(7),
            path: vec![0, 0]
        }],
        &turbo_html2pdf_core::build_cascade("", "", Default::default()),
        &options,
        &SnapshotInputs {
            width: 800.0,
            height: 600.0,
            fonts: &fonts,
            images: &ImageCtx {
                resolver: &turbo_html2pdf_core::NoImages,
                body_height: None
            }
        },
        &mut Diagnostics::default()
    )
    .err()
    .unwrap()
    .to_string()
    .contains("original"));
}
#[cfg(target_os = "macos")]
#[test]
fn original_script_fallback_ranges_match_chrome_without_changing_replacement_projection() {
    let control: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/utf16-text-geometry-chrome155.json")).unwrap();
    let mut fonts = FontRegistry::new();
    fonts.load_system_fonts();
    let original = snapshot(
        &[0x61, 0xd835, 0xdc00, 0xd800, 0x62, 0xdcff, 0x63],
        &fonts,
        "Times New Roman",
    )
    .unwrap();
    let replacement = snapshot(
        &[0x61, 0xd835, 0xdc00, 0xfffd, 0x62, 0xfffd, 0x63],
        &fonts,
        "Times New Roman",
    )
    .unwrap();
    for case in control["result"]["projectionDifferences"]
        .as_array()
        .unwrap()
    {
        let start = case["start"].as_u64().unwrap() as usize;
        let end = case["end"].as_u64().unwrap() as usize;
        for (snapshot, key) in [(&original, "original"), (&replacement, "replacement")] {
            let actual: Vec<_> = snapshot
                .text_range_boxes(SourceId(7), start, end)
                .unwrap()
                .into_iter()
                .map(|value| {
                    let r = value.transformed_bounds;
                    [r.x, r.y, r.width, r.height]
                })
                .collect();
            let expected: Vec<[f32; 4]> = serde_json::from_value(case[key].clone()).unwrap();
            assert_eq!(actual, expected, "{start}:{end} {key}");
        }
    }
}

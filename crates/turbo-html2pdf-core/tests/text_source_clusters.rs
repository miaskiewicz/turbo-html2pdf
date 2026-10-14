use turbo_html2pdf_core::layout::inline::{
    layout_paragraph_in, layout_paragraph_in_with_sources, InlineRun, Piece,
};
use turbo_html2pdf_core::layout::value::{Rgba, VAlign};
use turbo_html2pdf_core::text::Align;
use turbo_html2pdf_core::{Diagnostics, FontRegistry, NodeId};

fn run(registry: &FontRegistry, id: u32, text: &str) -> Piece {
    Piece::Run(InlineRun {
        node_id: NodeId(id),
        text: text.into(),
        face: registry
            .select(&["sans-serif"], 400, false)
            .unwrap()
            .clone(),
        families: vec!["sans-serif".into()],
        weight: 400,
        italic: false,
        font_size: 16.0,
        line_height: Some(24.0),
        letter_spacing: 0.0,
        color: Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        },
        valign: VAlign::Baseline,
        nowrap: false,
    })
}

#[test]
fn original_whitespace_unicode_and_wrapped_offsets_survive_actual_shaping() {
    let fonts = FontRegistry::new();
    let text = "  ab  é😀 cd";
    let pieces = [run(&fonts, 71, text)];
    let native = layout_paragraph_in_with_sources(
        &pieces,
        &fonts,
        Align::Left,
        &mut Diagnostics::default(),
        &|_| (0.0, 28.0),
        24.0,
    );
    assert!(native.layout.lines.len() >= 2);
    let mut offsets = Vec::new();
    for (line, clusters) in native.layout.lines.iter().zip(&native.clusters) {
        for (glyph_run, sources) in line.runs.iter().zip(clusters) {
            assert_eq!(glyph_run.node_id, NodeId(71));
            assert_eq!(glyph_run.glyphs.len(), sources.len());
            for source in sources {
                assert!(text.is_char_boundary(source.byte_start));
                assert!(text.is_char_boundary(source.byte_end));
                assert!(source.byte_start < source.byte_end);
                assert!(source.advance.is_finite());
                offsets.push((
                    source.byte_start,
                    source.byte_end,
                    source.utf16_start,
                    source.utf16_end,
                ));
            }
        }
    }
    assert_eq!(offsets[0], (2, 3, 2, 3));
    assert!(
        offsets.contains(&(6, 8, 6, 7)),
        "accent byte and UTF16 ranges differ"
    );
    assert!(
        offsets.contains(&(8, 12, 7, 9)),
        "astral scalar retains two original UTF16 units"
    );
    assert!(
        offsets.contains(&(13, 14, 10, 11)),
        "source offsets survive wrapping and collapsed spaces"
    );
}

#[test]
fn adjacent_input_runs_keep_distinct_source_ownership_and_original_offsets() {
    let fonts = FontRegistry::new();
    let pieces = [run(&fonts, 17, "ab"), run(&fonts, 99, "cd")];
    let native = layout_paragraph_in_with_sources(
        &pieces,
        &fonts,
        Align::Left,
        &mut Diagnostics::default(),
        &|_| (0.0, 200.0),
        24.0,
    );
    let runs = &native.layout.lines[0].runs;
    assert_eq!(
        runs.iter().map(|run| run.node_id).collect::<Vec<_>>(),
        vec![NodeId(17), NodeId(99)]
    );
    assert_eq!(native.clusters[0][0][0].utf16_start, 0);
    assert_eq!(native.clusters[0][1][0].utf16_start, 0);
    assert!(native.clusters[0][1][0].pen_x > native.clusters[0][0][0].pen_x);
}

#[test]
fn source_enabled_layout_preserves_the_existing_public_paragraph_output() {
    let fonts = FontRegistry::new();
    let pieces = [
        run(&fonts, 31, "alpha beta"),
        Piece::Break,
        run(&fonts, 72, "ffi é"),
    ];
    let plain = layout_paragraph_in(
        &pieces,
        &fonts,
        Align::Center,
        &mut Diagnostics::default(),
        &|_| (9.0, 45.0),
        24.0,
    );
    let source = layout_paragraph_in_with_sources(
        &pieces,
        &fonts,
        Align::Center,
        &mut Diagnostics::default(),
        &|_| (9.0, 45.0),
        24.0,
    );
    assert_eq!(
        (plain.width, plain.height),
        (source.layout.width, source.layout.height)
    );
    assert_eq!(plain.lines.len(), source.layout.lines.len());
    for (a, b) in plain.lines.iter().zip(&source.layout.lines) {
        assert_eq!(
            (a.width, a.top, a.height, a.indent),
            (b.width, b.top, b.height, b.indent)
        );
        assert_eq!(a.runs.len(), b.runs.len());
        for (a, b) in a.runs.iter().zip(&b.runs) {
            assert_eq!(a.node_id, b.node_id);
            assert_eq!(a.glyphs, b.glyphs);
            assert_eq!((a.font_size, a.color), (b.font_size, b.color));
        }
    }
}

fn capture_snapshot(
    texts: &[&str],
    width: f32,
    enabled: bool,
) -> turbo_html2pdf_core::layout_snapshot::LayoutSnapshot {
    use turbo_html2pdf_core::layout_snapshot::{
        snapshot_live_tree, snapshot_live_tree_with_text, SnapshotInputs, SourceId, SourceIdentity,
    };
    use turbo_html2pdf_core::{build_cascade, Attr, Element, ImageCtx, NoImages, Node, Tag};
    let nodes = [Node::Element(Element {
        tag: Tag::Html("div".into()),
        attrs: vec![Attr {
            name: "style".into(),
            value: format!(
                "width:{width}px;font-size:16px;line-height:24px;transform:translate(5px,7px)"
            ),
        }],
        children: texts
            .iter()
            .map(|text| Node::Text((*text).into()))
            .collect(),
    })];
    let mut ids = vec![SourceIdentity {
        id: SourceId(99),
        path: vec![0],
    }];
    ids.extend((0..texts.len()).map(|i| SourceIdentity {
        id: SourceId(i as u64 + 100),
        path: vec![0, i],
    }));
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let inputs = SnapshotInputs {
        width: 200.0,
        height: 200.0,
        fonts: &fonts,
        images: &images,
    };
    let css = build_cascade("", "", Default::default());
    if enabled {
        snapshot_live_tree_with_text(&nodes, &ids, &css, &inputs, &mut Diagnostics::default())
            .unwrap()
    } else {
        snapshot_live_tree(&nodes, &ids, &css, &inputs, &mut Diagnostics::default()).unwrap()
    }
}

#[test]
fn actual_snapshot_run_ids_distinguish_identical_wrapped_words_and_keep_source_and_transforms() {
    use turbo_html2pdf_core::layout_snapshot::{GeometryKind, SourceId};
    let native = capture_snapshot(&["ab ab"], 20.0, true);
    let runs = native.text_runs();
    assert_eq!(runs.len(), 2);
    assert!(runs
        .iter()
        .all(|run| run.source_id == Some(SourceId(100))
            && run.source_ancestors == vec![SourceId(99)]));
    assert_eq!(runs[0].glyphs[0].source.utf16_start, 0);
    assert_eq!(runs[1].glyphs[0].source.utf16_start, 3);
    let first = &native.fragments()[runs[0].fragment_index];
    let second = &native.fragments()[runs[1].fragment_index];
    assert_eq!(first.kind, GeometryKind::TextLineContainer);
    assert_ne!(first.layout_node_id, second.layout_node_id);
    assert_eq!(first.world_transform, [1.0, 0.0, 0.0, 1.0, 5.0, 7.0]);
    assert!(second.rect.y > first.rect.y);
    assert_eq!(
        native.source(SourceId(100)).unwrap().fragment_indices,
        vec![first.index, second.index]
    );
}

#[test]
fn existing_box_only_snapshots_keep_output_and_do_not_allocate_text_provenance() {
    let old = capture_snapshot(&["one two", "three"], 80.0, false);
    let traced = capture_snapshot(&["one two", "three"], 80.0, true);
    assert!(old.text_runs().is_empty());
    assert!(!traced.text_runs().is_empty());
    assert_eq!(old.fragments().len(), traced.fragments().len());
    for (a, b) in old.fragments().iter().zip(traced.fragments()) {
        assert_eq!(
            (a.source_id, &a.source_ancestors),
            (b.source_id, &b.source_ancestors)
        );
        assert_eq!(
            (a.rect, a.world_transform, a.transformed_bounds, a.used_box),
            (b.rect, b.world_transform, b.transformed_bounds, b.used_box)
        );
    }
}

#[test]
fn atom_cloning_and_positioning_preserve_real_nested_text_source_clusters() {
    use turbo_html2pdf_core::layout_snapshot::{
        snapshot_live_tree_with_text, SnapshotInputs, SourceId, SourceIdentity,
    };
    use turbo_html2pdf_core::{build_cascade, Attr, Element, ImageCtx, NoImages, Node, Tag};
    let nodes = [Node::Element(Element {
        tag: Tag::Html("div".into()),
        attrs: vec![],
        children: vec![
            Node::Text("left ".into()),
            Node::Element(Element {
                tag: Tag::Html("span".into()),
                attrs: vec![Attr {
                    name: "style".into(),
                    value: "display:inline-block;width:30px;height:24px".into(),
                }],
                children: vec![Node::Text("ab".into())],
            }),
            Node::Text(" right".into()),
        ],
    })];
    let ids = [
        SourceIdentity {
            id: SourceId(10),
            path: vec![0],
        },
        SourceIdentity {
            id: SourceId(11),
            path: vec![0, 0],
        },
        SourceIdentity {
            id: SourceId(12),
            path: vec![0, 1],
        },
        SourceIdentity {
            id: SourceId(13),
            path: vec![0, 1, 0],
        },
        SourceIdentity {
            id: SourceId(14),
            path: vec![0, 2],
        },
    ];
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let native = snapshot_live_tree_with_text(
        &nodes,
        &ids,
        &build_cascade("", "", Default::default()),
        &SnapshotInputs {
            width: 200.0,
            height: 100.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    )
    .unwrap();
    let atom = native
        .text_runs()
        .iter()
        .find(|run| run.source_id == Some(SourceId(13)))
        .unwrap();
    assert_eq!(atom.source_ancestors, vec![SourceId(10), SourceId(12)]);
    assert_eq!(atom.glyphs[0].source.utf16_start, 0);
    assert!(native.fragments()[atom.fragment_index].rect.x > 0.0);
}

#[test]
fn native_cluster_advances_match_independent_chrome_ranges_with_the_same_real_font() {
    use turbo_html2pdf_core::layout_snapshot::{
        snapshot_html_live_tree_with_text, SnapshotInputs, SourceId, SourceIdentity,
    };
    use turbo_html2pdf_core::{
        build_cascade, Attr, Element, FontFace, ImageCtx, NoImages, Node, Tag,
    };
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/text-source-chrome155.json")).unwrap();
    assert_eq!(
        oracle["fontSha256"],
        "197d9f3703b4c00af609178876a8d73e396f64fe438b2c871778566632374be3"
    );
    let mut fonts = FontRegistry::new();
    fonts.add(
        FontFace::from_bytes(
            include_bytes!("../assets/fonts/Go-Regular.ttf").to_vec(),
            "ParityGo",
            400,
            false,
        )
        .unwrap(),
    );
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    for case in oracle["result"]["cases"].as_array().unwrap() {
        // Chrome uses an installed emoji fallback outside the controlled Go face.
        // The source-offset regression covers astral text; this assertion certifies
        // actual controlled-face cluster advances, not unknown fallback metrics.
        if case["name"] == "astral-fallback" {
            continue;
        }
        let width = case["width"].as_f64().unwrap();
        let font_size = case["fontSize"].as_f64().unwrap_or(64.0);
        let line_height = case["lineHeight"].as_f64().unwrap_or(80.0);
        let transform = case["transform"].as_str().unwrap_or("none");
        let nodes=[Node::Element(Element{tag:Tag::Html("div".into()),attrs:vec![Attr{name:"style".into(),value:format!("font-family:ParityGo;font-size:{font_size}px;line-height:{line_height}px;width:{width}px;transform:{transform};transform-origin:0 0")}],children:vec![Node::Text(case["text"].as_str().unwrap().into())]})];
        let ids = [
            SourceIdentity {
                id: SourceId(99),
                path: vec![0],
            },
            SourceIdentity {
                id: SourceId(100),
                path: vec![0, 0],
            },
        ];
        let native = snapshot_html_live_tree_with_text(
            &nodes,
            &ids,
            &build_cascade("", "", Default::default()),
            &SnapshotInputs {
                width: 400.0,
                height: 300.0,
                fonts: &fonts,
                images: &images,
            },
            &mut Diagnostics::default(),
        )
        .unwrap();
        assert_cluster_geometry(&native, case);
    }
}

fn assert_cluster_geometry(
    native: &turbo_html2pdf_core::layout_snapshot::LayoutSnapshot,
    case: &serde_json::Value,
) {
    for box_ in native.text_cluster_boxes() {
        let start = box_.utf16_start;
        let end = box_.utf16_end;
        let expected = &case["ranges"][format!("{start}:{end}")]["rect"];
        let actual = box_.transformed_bounds;
        assert_eq!(
            (
                f64::from(actual.x),
                f64::from(actual.y),
                f64::from(actual.width),
                f64::from(actual.height)
            ),
            (
                expected["x"].as_f64().unwrap(),
                expected["y"].as_f64().unwrap(),
                expected["width"].as_f64().unwrap(),
                expected["height"].as_f64().unwrap()
            ),
            "{} genuine native cluster {start}:{end}",
            case["name"]
        );
    }
}

#[test]
fn text_snapshot_errors_keep_the_original_source_and_viewport_validation() {
    use turbo_html2pdf_core::layout_snapshot::{
        snapshot_live_tree_with_text, SnapshotError, SnapshotInputs, SourceId, SourceIdentity,
        TextSnapshotError,
    };
    use turbo_html2pdf_core::{build_cascade, ImageCtx, NoImages, Node};
    let nodes = [Node::Text("ab".into())];
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let inputs = SnapshotInputs {
        width: 200.0,
        height: 100.0,
        fonts: &fonts,
        images: &images,
    };
    let ids = [
        SourceIdentity {
            id: SourceId(9),
            path: vec![0],
        },
        SourceIdentity {
            id: SourceId(9),
            path: vec![0],
        },
    ];
    let css = build_cascade("", "", Default::default());
    assert!(matches!(
        snapshot_live_tree_with_text(&nodes, &ids, &css, &inputs, &mut Diagnostics::default()),
        Err(TextSnapshotError::Snapshot(SnapshotError::DuplicateId(
            SourceId(9)
        )))
    ));
    let invalid = SnapshotInputs {
        width: 0.0,
        ..inputs
    };
    assert!(matches!(
        snapshot_live_tree_with_text(&nodes, &[], &css, &invalid, &mut Diagnostics::default()),
        Err(TextSnapshotError::Snapshot(SnapshotError::InvalidViewport))
    ));
    let unmapped =
        snapshot_live_tree_with_text(&nodes, &[], &css, &inputs, &mut Diagnostics::default())
            .unwrap();
    assert_eq!(unmapped.text_runs()[0].source_id, None);
    assert!(unmapped.text_runs()[0].source_ancestors.is_empty());
}

#[test]
fn source_metadata_preserves_distinct_baselines_with_identical_public_run_identity() {
    let fonts = FontRegistry::new();
    let mut raised = run(&fonts, 17, "b");
    if let Piece::Run(value) = &mut raised {
        value.valign = VAlign::Super;
    }
    let pieces = [run(&fonts, 17, "a"), raised];
    let native = layout_paragraph_in_with_sources(
        &pieces,
        &fonts,
        Align::Left,
        &mut Diagnostics::default(),
        &|_| (0.0, 200.0),
        24.0,
    );
    assert_eq!(native.layout.lines[0].runs.len(), 2);
    let metrics = &native.font_boxes[0];
    assert_eq!(metrics.len(), 2);
    assert!(metrics[1].baseline < metrics[0].baseline);
    for (run, metric) in native.layout.lines[0].runs.iter().zip(metrics) {
        assert_eq!(run.glyphs[0].y, metric.baseline);
    }
}

#[test]
fn font_alias_does_not_select_a_foreign_legacy_metric_adjustment() {
    use turbo_html2pdf_core::FontFace;
    let face = FontFace::from_bytes(
        include_bytes!("../assets/fonts/Go-Regular.ttf").to_vec(),
        "ParityGo",
        400,
        false,
    )
    .unwrap();
    let renamed = face.with_family("Helvetica");
    for size in [16.0, 32.0, 64.0, 96.0] {
        assert_eq!(face.html_metrics(size), renamed.html_metrics(size));
    }
}

#[test]
fn actual_text_source_keeps_collapsed_whitespace_and_distinct_owner_inputs() {
    use turbo_html2pdf_core::layout_snapshot::SourceId;
    let native = capture_snapshot(&["  a  b ", "é😀"], 80.0, true);
    let inputs = native.text_sources();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].source_id, SourceId(100));
    assert_eq!(inputs[0].text, "  a  b ");
    assert_eq!(inputs[1].source_id, SourceId(101));
    assert_eq!(inputs[1].text.encode_utf16().count(), 3);
    assert!(capture_snapshot(&["  a  b "], 80.0, false)
        .text_sources()
        .is_empty());
}

#[test]
fn whitespace_only_source_bytes_have_a_bound_before_native_layout_capture() {
    use turbo_html2pdf_core::layout_snapshot::{
        snapshot_styled_tree_with_text, SnapshotInputs, TextSnapshotError,
    };
    use turbo_html2pdf_core::{ImageCtx, NoImages, StyledNode};
    let nodes = [StyledNode::Text(" ".repeat(16 * 1024 * 1024 + 1))];
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let result = snapshot_styled_tree_with_text(
        &nodes,
        &[],
        &SnapshotInputs {
            width: 200.0,
            height: 200.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    );
    assert!(matches!(result, Err(TextSnapshotError::Budget)));
}

#[test]
fn source_clusters_preserve_native_shaper_direction_without_claiming_bidi_itemization() {
    let fonts = FontRegistry::new();
    let pieces = [run(&fonts, 17, "ab"), Piece::Break, run(&fonts, 99, "אב")];
    let native = layout_paragraph_in_with_sources(
        &pieces,
        &fonts,
        Align::Left,
        &mut Diagnostics::default(),
        &|_| (0.0, 200.0),
        24.0,
    );
    assert!(native.clusters[0]
        .iter()
        .flatten()
        .all(|c| !c.right_to_left));
    assert!(!native.clusters[1].is_empty());
    assert!(native.clusters[1].iter().flatten().all(|c| c.right_to_left));
}

#[test]
fn source_character_work_has_a_bound_even_below_the_byte_budget() {
    use turbo_html2pdf_core::layout_snapshot::{
        snapshot_styled_tree_with_text, SnapshotInputs, TextSnapshotError,
    };
    use turbo_html2pdf_core::{ImageCtx, NoImages, StyledNode};
    let nodes = [StyledNode::Text("é".repeat(1_000_001))];
    let fonts = FontRegistry::new();
    let images = ImageCtx {
        resolver: &NoImages,
        body_height: None,
    };
    let result = snapshot_styled_tree_with_text(
        &nodes,
        &[],
        &SnapshotInputs {
            width: 200.0,
            height: 200.0,
            fonts: &fonts,
            images: &images,
        },
        &mut Diagnostics::default(),
    );
    assert!(matches!(result, Err(TextSnapshotError::Budget)));
}

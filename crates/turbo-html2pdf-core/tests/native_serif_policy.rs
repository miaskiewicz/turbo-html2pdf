#![cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
use turbo_html2pdf_core::layout_snapshot::{
    snapshot_html_live_tree_with_native_text, LayoutSnapshot, NativeTextOptions, SnapshotInputs,
    SourceId, SourceIdentity,
};
use turbo_html2pdf_core::{
    build_cascade, parse_html, Diagnostics, FontRegistry, ImageCtx, NoImages,
};

#[test]
fn native_serif_primary_is_physical_times_while_pdf_keeps_its_existing_alias() {
    let mut html = FontRegistry::new();
    html.load_system_fonts_for_html();
    let mut pdf = FontRegistry::new();
    pdf.load_system_fonts();
    let actual = html.select(&["serif"], 400, false).unwrap();
    let times = html.select(&["Times"], 400, false).unwrap();
    let previous = pdf.select(&["serif"], 400, false).unwrap();
    let roman = pdf.select(&["Times New Roman"], 400, false).unwrap();
    assert_eq!(actual.data(), times.data());
    assert_eq!(actual.index(), times.index());
    assert_eq!(previous.data(), roman.data());
    assert_eq!(previous.index(), roman.index());
    assert_ne!(actual.data(), previous.data());
}

#[test]
fn native_serif_and_explicit_physical_font_ranges_match_fresh_chrome() {
    let control: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-serif/chrome155.json")).unwrap();
    assert_eq!(control["errors"], serde_json::json!([]));
    assert_eq!(
        control["probeSha256"],
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("fixtures/native-serif/probe.js"))
        )
    );
    let mut fonts = FontRegistry::new();
    fonts.load_system_fonts_for_html();
    for case in control["result"].as_array().unwrap() {
        compare_font_case(&fonts, case);
    }
}
fn compare_font_case(fonts: &FontRegistry, case: &serde_json::Value) {
    let family = case["family"].as_str().unwrap();
    let size = case["size"].as_f64().unwrap() as f32;
    let face = fonts.select(&[family], 400, false).unwrap();
    let metrics = face.html_metrics(size * 2.0);
    assert_eq!(
        metrics.ascent as f64,
        case["fontBoundingBoxAscent"].as_f64().unwrap(),
        "{family} {size}"
    );
    assert_eq!(
        metrics.descent as f64,
        case["fontBoundingBoxDescent"].as_f64().unwrap(),
        "{family} {size}"
    );
    assert_eq!(
        face.measure("light", size * 2.0, 0.0) as f64,
        case["width"].as_f64().unwrap(),
        "{family} {size}"
    );
    let snapshot = snapshot(fonts, family, size);
    let expected: [f64; 4] = serde_json::from_value(case["range"].clone()).unwrap();
    let ranges = snapshot.text_range_boxes(SourceId(2), 0, 5).unwrap();
    assert_eq!(ranges.len(), 1);
    assert_eq!(rect(ranges[0].rect), expected, "{family} {size}");
    let own = snapshot.source(SourceId(1)).unwrap().fragment_indices[0];
    let expected: [f64; 4] = serde_json::from_value(case["box"].clone()).unwrap();
    assert_eq!(
        rect(snapshot.fragments()[own].rect),
        expected,
        "{family} {size} line box"
    );
}
fn rect(value: turbo_html2pdf_core::layout_snapshot::Rect) -> [f64; 4] {
    [
        value.x as f64,
        value.y as f64,
        value.width as f64,
        value.height as f64,
    ]
}
fn snapshot(fonts: &FontRegistry, family: &str, size: f32) -> LayoutSnapshot {
    let family = match family {
        "serif" => family.to_owned(),
        _ => format!("\"{family}\""),
    };
    let nodes = parse_html(&format!(
        "<div style='font-family:{family};font-size:{size}px;line-height:normal'>light</div>"
    ))
    .unwrap();
    snapshot_html_live_tree_with_native_text(
        &nodes,
        &[
            SourceIdentity {
                id: SourceId(1),
                path: vec![0],
            },
            SourceIdentity {
                id: SourceId(2),
                path: vec![0, 0],
            },
        ],
        &build_cascade("", "", Default::default()),
        &NativeTextOptions {
            original_text: &[],
            svg_viewports: &[],
            layout_scale: 2.0,
        },
        &SnapshotInputs {
            width: 1200.0,
            height: 600.0,
            fonts,
            images: &ImageCtx {
                resolver: &NoImages,
                body_height: None,
            },
        },
        &mut Diagnostics::default(),
    )
    .unwrap()
}

//! Hot-path instrumentation tests (Phase 13). Compiled only with the `perf`
//! feature; run with `cargo test -p turbo-html2pdf-core --features perf`. They assert
//! the `FontFace` caches actually collapse the hot paths (one parse per distinct
//! char, one shape per distinct run) rather than per occurrence.
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/perf.rs`; the ones here need TKind directives, templating or the PDF emitter.
#![cfg(feature = "perf")]

mod common;

use turbo_html2pdf_core::perf;

/// Lay out a 1000-row, 3-column table whose cells come from `cell(row, column)` and
/// return the `font.shape` count (rustybuzz calls: shape-cache misses).
fn table_shape_count(label: &str, cell: impl Fn(usize, usize) -> String) -> u64 {
    use turbo_html2pdf_core::style::TokenSet;
    use turbo_html2pdf_core::{build_cascade, compile, layout, CompileOptions, Diagnostics};
    let tmpl = "<table>{% for r in rows %}<tr><td>{{ r.a }}</td><td>{{ r.b }}</td>\
                <td>{{ r.c }}</td></tr>{% endfor %}</table>";
    let rows: Vec<_> = (0..1000)
        .map(|i| serde_json::json!({"a": cell(i, 0), "b": cell(i, 1), "c": cell(i, 2)}))
        .collect();
    let data = serde_json::json!({ "rows": rows });
    let (program, _) = compile(tmpl, &CompileOptions::default()).unwrap();
    let (flow, _) = program.render_nodes(&data, Some(0)).unwrap();
    let cascade = build_cascade("", "", TokenSet::default());
    let styled = flow.style(&cascade);
    let reg = common::registry();
    perf::reset();
    let mut diags = Diagnostics::default();
    let _ = layout(&styled, 600.0, &reg, &mut diags);
    eprintln!("HOTPATH {label} layout: {:?}", perf::snapshot());
    perf::count("font.shape")
}

/// Distinct cells: every run is shaped at most twice — once for the table's width
/// measurement, once for line layout. The shape cache is bounded (128 entries per face,
/// turbo-dom-layout `text::font`), so with 3000 distinct runs the second pass misses;
/// the old `< 5000` budget predates that bound and was never run in CI
/// (miaskiewicz/turbo-html2pdf#28: 6096 on every platform).
#[test]
fn report_hot_path_breakdown() {
    let distinct = table_shape_count("report-1k", |i, c| match c {
        0 => format!("Item {i}"),
        1 => i.to_string(),
        _ => format!("{i}.50"),
    });
    assert!(distinct <= 2 * 3000 + 100, "font.shape {distinct}");
}

/// Repeated cells: the cache collapses them — one shape per distinct run (the regression
/// the budget guards), not one per occurrence.
#[test]
fn repeated_runs_shape_once() {
    let repeated = table_shape_count("repeat-1k", |_, c| ["Item", "42", "1.50"][c].to_string());
    assert!(repeated <= 10, "font.shape {repeated}");
}

#[test]
fn invoice_stage_timing() {
    use std::time::Instant;
    use turbo_html2pdf_core::style::TokenSet;
    use turbo_html2pdf_core::{build_cascade, compile, layout, CompileOptions, Diagnostics};
    let tmpl = "<style>.tot{font-weight:bold}</style><h1>Invoice {{ n }}</h1>\
        <table>{% for r in rows %}<tr><td>{{ r.d }}</td><td>{{ r.q }}</td>\
        <td>{{ r.p }}</td></tr>{% endfor %}</table><p class=tot>Total {{ t }}</p>";
    let rows: Vec<_> = (0..8)
        .map(|i| serde_json::json!({"d": format!("Service {i}"), "q": i, "p": format!("{i}.00")}))
        .collect();
    let data = serde_json::json!({ "n": 4012, "rows": rows, "t": "1234.00" });
    let time = |iters: u32, f: &mut dyn FnMut()| {
        let t = Instant::now();
        for _ in 0..iters {
            f();
        }
        t.elapsed().as_micros() as f64 / f64::from(iters)
    };
    let t_reg = time(200, &mut || {
        std::hint::black_box(common::registry());
    });
    let (program, _) = compile(tmpl, &CompileOptions::default()).unwrap();
    let t_render = time(2000, &mut || {
        std::hint::black_box(program.render_nodes(&data, Some(0)).unwrap());
    });
    let t_cascade = time(2000, &mut || {
        std::hint::black_box(build_cascade("", "", TokenSet::default()));
    });
    let (flow, _) = program.render_nodes(&data, Some(0)).unwrap();
    let cascade = build_cascade("", "", TokenSet::default());
    let t_style = time(2000, &mut || {
        std::hint::black_box(flow.style(&cascade));
    });
    let styled = flow.style(&cascade);
    let reg = common::registry();
    let t_layout = time(2000, &mut || {
        let mut d = Diagnostics::default();
        std::hint::black_box(layout(&styled, 600.0, &reg, &mut d));
    });
    eprintln!(
        "STAGES(us): registry={t_reg:.1} render_nodes={t_render:.1} cascade={t_cascade:.1} style={t_style:.1} layout={t_layout:.1}"
    );
}

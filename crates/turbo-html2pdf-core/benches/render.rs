//! Criterion benchmark harness for the rendering pipeline that exists today
//! (Phase 13 groundwork).
//!
//! Each fixture is timed at three stages of the public pipeline:
//!
//! 1. `compile` — parse the template into a reusable `Program`.
//! 2. `render_nodes` — render against data and parse into the document flow.
//! 3. `compile_to_galley` — the whole chain: compile, render_nodes, build the
//!    style cascade, style the flow, and lay it out into the galley `Fragment`.
//!
//! Determinism is pinned with `now = Some(0)` on every `render_nodes` call and
//! the fonts are loaded from turbo-dom-layout's `assets/fonts/` (never a system path),
//! so a run never depends on the wall clock, randomness, or the host.
//!
//! The galley `Fragment` is the end of the pipeline that exists at the time of
//! writing.
//
// TODO(phase9): extend to full PDF emit once the emitter lands. Add a fourth
// stage that takes the galley `Fragment` through the fragmenter/paginator and
// the PDF emitter, and measure end-to-end document bytes.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use turbo_html2pdf_core::style::TokenSet;
use turbo_html2pdf_core::{
    build_cascade, compile, layout, CompileOptions, Diagnostics, FontRegistry,
};

#[path = "fixtures.rs"]
mod fixtures;

/// Pinned clock for deterministic renders (§3.3): never the wall clock.
const NOW: Option<i64> = Some(0);

/// Content-box width passed to `layout`, matching the integration tests.
const CB_WIDTH: f32 = 600.0;

/// Compile-only: the work `compile` does (template + partials parse + cache).
fn bench_compile(c: &mut Criterion) {
    let mut group = c.benchmark_group("compile");
    for fx in fixtures::all() {
        group.bench_with_input(BenchmarkId::from_parameter(fx.name), &fx, |b, fx| {
            b.iter(|| {
                let (program, _) = compile(
                    std::hint::black_box(&fx.template),
                    &CompileOptions::default(),
                )
                .expect("compile");
                std::hint::black_box(program);
            });
        });
    }
    group.finish();
}

/// render_nodes: render the precompiled `Program` against data and parse it
/// into its document flow. The clock is pinned with `NOW`.
fn bench_render_nodes(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_nodes");
    for fx in fixtures::all() {
        let (program, _) = compile(&fx.template, &CompileOptions::default()).expect("compile");
        group.bench_with_input(BenchmarkId::from_parameter(fx.name), &fx, |b, fx| {
            b.iter(|| {
                let (flow, _) = program
                    .render_nodes(std::hint::black_box(&fx.data), NOW)
                    .expect("render_nodes");
                std::hint::black_box(flow);
            });
        });
    }
    group.finish();
}

/// Full pipeline: compile -> render_nodes -> build_cascade -> Flow::style ->
/// layout, ending at the galley `Fragment`. The font registry is built once
/// (font parsing is not part of the measured pipeline).
fn bench_compile_to_galley(c: &mut Criterion) {
    let registry: FontRegistry = fixtures::registry();
    let mut group = c.benchmark_group("compile_to_galley");
    for fx in fixtures::all() {
        // Approximate per-fixture work so Criterion can report throughput in
        // rows/elements; reuse the same metric (data byte length) for all.
        group.throughput(Throughput::Bytes(fx.data.to_string().len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(fx.name), &fx, |b, fx| {
            b.iter(|| {
                let (program, _) =
                    compile(&fx.template, &CompileOptions::default()).expect("compile");
                let (flow, _) = program.render_nodes(&fx.data, NOW).expect("render_nodes");
                let cascade = build_cascade("", "", TokenSet::default());
                let styled = flow.style(&cascade);
                let mut diags = Diagnostics::default();
                let galley = layout(&styled, CB_WIDTH, &registry, &mut diags);
                std::hint::black_box(galley);
            });
        });
    }
    group.finish();
}

/// Cold single-run: the realistic one-PDF case. Unlike `compile_to_galley`, the
/// font registry is rebuilt *inside* the measured loop, so font parsing and a
/// cold glyph/shape cache are part of the cost — this is what a process renders
/// for a single document, the case worth optimizing.
fn bench_cold_single_run(c: &mut Criterion) {
    let mut group = c.benchmark_group("cold_single_run");
    for fx in fixtures::all() {
        group.throughput(Throughput::Bytes(fx.data.to_string().len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(fx.name), &fx, |b, fx| {
            b.iter(|| {
                let registry: FontRegistry = fixtures::registry();
                let (program, _) =
                    compile(&fx.template, &CompileOptions::default()).expect("compile");
                let (flow, _) = program.render_nodes(&fx.data, NOW).expect("render_nodes");
                let cascade = build_cascade("", "", TokenSet::default());
                let styled = flow.style(&cascade);
                let mut diags = Diagnostics::default();
                let galley = layout(&styled, CB_WIDTH, &registry, &mut diags);
                std::hint::black_box(galley);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_compile,
    bench_render_nodes,
    bench_compile_to_galley,
    bench_cold_single_run
);
criterion_main!(benches);

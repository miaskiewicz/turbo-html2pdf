//! Multi-run inline layout (§5.2, AC-5.4): wrapping across runs, baseline
//! alignment of mixed sizes, vertical-align, alignment, per-glyph fallback, and
//! the `.notdef` lint.

mod common;

use turbo_html2pdf_core::layout::fragment::NodeId;
use turbo_html2pdf_core::layout::inline::*;
use turbo_html2pdf_core::layout::value::{Rgba, VAlign};
use turbo_html2pdf_core::text::{Align, FontFace, FontRegistry};
use turbo_html2pdf_core::{Diagnostics, LintCode};

fn run(text: &str, face: &FontFace) -> InlineRun {
    InlineRun {
        node_id: NodeId(0),
        text: text.to_string(),
        face: face.clone(),
        families: vec![face.family().to_string()],
        weight: face.weight(),
        italic: face.is_italic(),
        font_size: 16.0,
        line_height: None,
        letter_spacing: 0.0,
        color: Rgba::BLACK,
        valign: VAlign::Baseline,
        nowrap: false,
    }
}

fn lay(runs: &[InlineRun], max_width: f32, align: Align) -> (ParagraphLayout, Diagnostics) {
    let reg = common::registry();
    let mut diags = Diagnostics::default();
    let p = layout_runs(runs, &reg, max_width, align, &mut diags);
    (p, diags)
}

fn first_x(line: &InlineLine) -> f32 {
    line.runs[0].glyphs[0].x
}

#[test]
fn single_run_one_line() {
    let (p, diags) = lay(
        &[run("hello world", &common::evolventa())],
        1000.0,
        Align::Left,
    );
    assert_eq!(p.lines.len(), 1);
    assert!(p.width > 0.0);
    assert_eq!(p.height, p.lines[0].height);
    assert!(diags.is_empty());
}

#[test]
fn empty_and_whitespace_produce_no_lines() {
    assert!(lay(&[run("", &common::evolventa())], 500.0, Align::Left)
        .0
        .lines
        .is_empty());
    let ws = lay(&[run("   ", &common::evolventa())], 500.0, Align::Left).0;
    assert!(ws.lines.is_empty());
    assert_eq!(ws.height, 0.0);
}

#[test]
fn wraps_to_multiple_lines_and_stacks() {
    let (p, _) = lay(
        &[run("aaaa bbbb cccc dddd eeee", &common::evolventa())],
        80.0,
        Align::Left,
    );
    assert!(p.lines.len() >= 2);
    // finalize stacks lines: second line top equals first line height.
    assert_eq!(p.lines[1].top, p.lines[0].height);
    assert_eq!(p.height, p.lines.iter().map(|l| l.height).sum::<f32>());
}

#[test]
fn long_word_takes_its_own_line() {
    let (p, _) = lay(
        &[run("supercalifragilistic short", &common::evolventa())],
        40.0,
        Align::Left,
    );
    assert!(p.lines.len() >= 2);
}

#[test]
fn leading_and_trailing_whitespace_collapse() {
    let (p, _) = lay(&[run("  a b  ", &common::evolventa())], 1000.0, Align::Left);
    assert_eq!(p.lines.len(), 1);
}

#[test]
fn multi_run_word_keeps_distinct_faces() {
    // "bo" + "ld" with no space -> one word, two segments, two glyph runs
    // (different weight, so not merged).
    let runs = [
        run("bo", &common::evolventa()),
        run("ld", &common::evolventa_bold()),
    ];
    let (p, _) = lay(&runs, 1000.0, Align::Left);
    assert_eq!(p.lines.len(), 1);
    assert_eq!(p.lines[0].runs.len(), 2);
}

#[test]
fn adjacent_identical_runs_merge() {
    let runs = [
        run("ab", &common::evolventa()),
        run("cd", &common::evolventa()),
    ];
    let (p, _) = lay(&runs, 1000.0, Align::Left);
    assert_eq!(p.lines[0].runs.len(), 1); // merged into one run
    assert_eq!(p.lines[0].runs[0].glyphs.len(), 4);
}

#[test]
fn mixed_font_sizes_grow_line_box() {
    let small = lay(&[run("a", &common::evolventa())], 1000.0, Align::Left).0;
    let mut big_run = run("A", &common::evolventa());
    big_run.font_size = 40.0;
    let mixed = lay(
        &[run("a", &common::evolventa()), big_run],
        1000.0,
        Align::Left,
    )
    .0;
    assert!(mixed.lines[0].height > small.lines[0].height);
}

#[test]
fn superscript_raises_subscript_lowers() {
    let mut sup = run("y", &common::evolventa());
    sup.valign = VAlign::Super;
    let p = lay(&[run("x", &common::evolventa()), sup], 1000.0, Align::Left).0;
    let glyphs = &p.lines[0].runs[0].glyphs; // merged: baseline glyph then raised glyph
    assert!(glyphs[1].y < glyphs[0].y);

    let mut sub = run("y", &common::evolventa());
    sub.valign = VAlign::Sub;
    let q = lay(&[run("x", &common::evolventa()), sub], 1000.0, Align::Left).0;
    let g = &q.lines[0].runs[0].glyphs;
    assert!(g[1].y > g[0].y);
}

#[test]
fn alignment_shifts_line() {
    let text = "hi";
    let left = lay(&[run(text, &common::evolventa())], 500.0, Align::Left).0;
    let right = lay(&[run(text, &common::evolventa())], 500.0, Align::Right).0;
    let center = lay(&[run(text, &common::evolventa())], 500.0, Align::Center).0;
    let justify = lay(&[run(text, &common::evolventa())], 500.0, Align::Justify).0;
    assert!(first_x(&left.lines[0]) < first_x(&center.lines[0]));
    assert!(first_x(&center.lines[0]) < first_x(&right.lines[0]));
    assert_eq!(first_x(&justify.lines[0]), first_x(&left.lines[0])); // justify lays out left in v1
}

#[test]
fn letter_spacing_widens() {
    let plain = lay(&[run("mmmm", &common::evolventa())], 1000.0, Align::Left).0;
    let mut spaced_run = run("mmmm", &common::evolventa());
    spaced_run.letter_spacing = 5.0;
    let spaced = lay(&[spaced_run], 1000.0, Align::Left).0;
    assert!(spaced.lines[0].width > plain.lines[0].width);
}

#[test]
fn explicit_line_height_sets_box() {
    let mut r = run("a", &common::evolventa());
    r.line_height = Some(60.0);
    let p = lay(&[r], 1000.0, Align::Left).0;
    assert_eq!(p.lines[0].height, 60.0);
}

#[test]
fn missing_glyph_emits_notdef_lint() {
    let (_p, diags) = lay(&[run("中", &common::evolventa())], 500.0, Align::Left);
    assert!(diags.lints.iter().any(|l| l.code == LintCode::NotdefGlyph));
}

#[test]
fn per_glyph_fallback_finds_another_face() {
    let go = common::go();
    let ev = common::evolventa();
    let evb = common::evolventa_bold();
    let probe = (0x80u32..0x10000)
        .filter_map(char::from_u32)
        .find(|&c| !go.has_glyph(c) && (ev.has_glyph(c) || evb.has_glyph(c)));
    let c = probe.expect("expected a char Go lacks but Evolventa has");

    let mut r = run(&c.to_string(), &go);
    r.families = vec!["Go".to_string()];
    let reg = {
        let mut reg = FontRegistry::new();
        reg.add(go.clone());
        reg.add(ev);
        reg
    };
    let mut diags = Diagnostics::default();
    let p = layout_runs(&[r], &reg, 1000.0, Align::Left, &mut diags);
    assert!(diags.is_empty()); // fallback found, no notdef
    assert_eq!(p.lines[0].runs[0].face.family(), "Evolventa");
}

#[test]
fn nowrap_run_stays_on_one_line() {
    // `white-space:nowrap` folds inter-word spaces into the word so the text never
    // wraps, even when the column is far too narrow (menu tabs, buttons).
    let mut r = run("View source history", &common::evolventa());
    r.nowrap = true;
    // A width of 10px would wrap every word without nowrap.
    let (layout, _) = lay(&[r], 10.0, Align::Left);
    assert_eq!(layout.lines.len(), 1, "nowrap text must not wrap");
    let wrapping = run("View source history", &common::evolventa());
    let (wl, _) = lay(&[wrapping], 10.0, Align::Left);
    assert!(
        wl.lines.len() > 1,
        "control: wrapping text does wrap at 10px"
    );
}

fn atom(id: usize, valign: VAlign, height: f32) -> InlineAtom {
    InlineAtom {
        id,
        width: 20.0,
        height,
        valign,
        margin_top: 0.0,
        margin_bottom: 0.0,
        margin_left: 0.0,
        margin_right: 0.0,
    }
}

#[test]
fn vertical_align_places_inline_block_atoms() {
    // A 60px baseline atom sets the line box height; three 40px atoms then pin
    // differently: `top` to the line top (y=0), `bottom` to the line bottom
    // (60 - 40 = 20), and `middle` centred ((60 - 40)/2 = 10).
    let reg = common::registry();
    let mut diags = Diagnostics::default();
    let pieces = vec![
        Piece::Atom(atom(0, VAlign::Baseline, 60.0)),
        Piece::Atom(atom(1, VAlign::Top, 40.0)),
        Piece::Atom(atom(2, VAlign::Bottom, 40.0)),
        Piece::Atom(atom(3, VAlign::Middle, 40.0)),
    ];
    let p = layout_paragraph(&pieces, &reg, 500.0, Align::Left, &mut diags);
    let line = &p.lines[0];
    assert!(
        (line.height - 60.0).abs() < 0.01,
        "line height, got {}",
        line.height
    );
    let y = |id: usize| line.atoms.iter().find(|a| a.id == id).unwrap().y;
    assert!(
        (y(1) - 0.0).abs() < 0.01,
        "top pins to line top, got {}",
        y(1)
    );
    assert!(
        (y(2) - 20.0).abs() < 0.01,
        "bottom pins to line bottom, got {}",
        y(2)
    );
    assert!((y(3) - 10.0).abs() < 0.01, "middle centres, got {}", y(3));
}

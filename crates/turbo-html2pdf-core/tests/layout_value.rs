//! Typed value resolution (§5/§4.1): length/color parsing and `BoxStyle`
//! resolution from a `ComputedStyle`. One assertion family per branch so the
//! coverage gate stays honest.

use turbo_html2pdf_core::layout::value::*;
use turbo_html2pdf_core::text::{Align, WhiteSpace};
use turbo_html2pdf_core::ComputedStyle;

fn style(pairs: &[(&str, &str)]) -> ComputedStyle {
    ComputedStyle::from_pairs(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
}

fn ctx() -> ResolveCtx {
    ResolveCtx {
        parent_font_size: 16.0,
        cb_width: 200.0,
    }
}

fn resolve(pairs: &[(&str, &str)]) -> BoxStyle {
    resolve_box_style(&style(pairs), ctx())
}

fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

// ---------------------------------------------------------------- primitives

#[test]
fn edges_helpers() {
    let e = Edges::all(4.0);
    assert_eq!(
        e,
        Edges {
            top: 4.0,
            right: 4.0,
            bottom: 4.0,
            left: 4.0
        }
    );
    approx(e.horizontal(), 8.0);
    approx(e.vertical(), 8.0);
    assert_eq!(Edges::default(), Edges::all(0.0));
}

#[test]
fn length_pct_resolves() {
    assert_eq!(LengthPct::Auto.resolve(200.0), None);
    assert_eq!(LengthPct::Px(5.0).resolve(200.0), Some(5.0));
    assert_eq!(LengthPct::Pct(50.0).resolve(200.0), Some(100.0));
}

#[test]
fn parse_px_units() {
    approx(parse_px("16px", 16.0).unwrap(), 16.0);
    approx(parse_px("5", 16.0).unwrap(), 5.0);
    approx(parse_px("12pt", 16.0).unwrap(), 16.0);
    approx(parse_px("1pc", 16.0).unwrap(), 16.0);
    approx(parse_px("1in", 16.0).unwrap(), 96.0);
    approx(parse_px("2.54cm", 16.0).unwrap(), 96.0);
    approx(parse_px("25.4mm", 16.0).unwrap(), 96.0);
    approx(parse_px("2em", 16.0).unwrap(), 32.0);
}

#[test]
fn rem_is_root_relative_not_parent_relative() {
    // `em` scales with the (parent) font size; `rem` is fixed to the 16px root, so
    // a `1rem` label inside a shrunk-font ancestor stays 16px instead of compounding
    // down to unreadable (the Wikipedia appearance-panel radio labels).
    approx(parse_px("1rem", 8.0).unwrap(), 16.0);
    approx(parse_px("1rem", 40.0).unwrap(), 16.0);
    approx(parse_px("2em", 8.0).unwrap(), 16.0);
    assert_eq!(parse_length_pct("1.5rem", 8.0), Some(LengthPct::Px(24.0)));
}

#[test]
fn parse_calc_additive_lengths() {
    // Additive calc() of px/em terms (Codex component sizing, Vector breakpoints).
    approx(parse_px("calc(100px + 20px)", 16.0).unwrap(), 120.0);
    approx(parse_px("calc(1rem + 10px)", 16.0).unwrap(), 26.0);
    approx(parse_px("calc(1120px - 1px)", 16.0).unwrap(), 1119.0);
    approx(parse_px("calc(2em - 8px)", 16.0).unwrap(), 24.0);
    // As a length-percentage too (width/padding/etc.).
    assert_eq!(
        parse_length_pct("calc(1rem + 4px)", 16.0),
        Some(LengthPct::Px(20.0))
    );
    // Multiplicative / percentage calc isn't handled here → None (caller defaults).
    assert_eq!(parse_px("calc(2 * 10px)", 16.0), None);
    assert_eq!(parse_px("calc(100% - 20px)", 16.0), None);
}

#[test]
fn parse_px_rejects_pct_and_garbage() {
    assert_eq!(parse_px("50%", 16.0), None);
    assert_eq!(parse_px("5xx", 16.0), None);
    assert_eq!(parse_px("abc", 16.0), None);
    assert_eq!(parse_px("", 16.0), None);
}

#[test]
fn parse_length_pct_variants() {
    assert_eq!(parse_length_pct("auto", 16.0), Some(LengthPct::Auto));
    assert_eq!(parse_length_pct("50%", 16.0), Some(LengthPct::Pct(50.0)));
    assert_eq!(parse_length_pct("10px", 16.0), Some(LengthPct::Px(10.0)));
    assert_eq!(parse_length_pct("2em", 16.0), Some(LengthPct::Px(32.0)));
    assert_eq!(parse_length_pct("xyz", 16.0), None);
}

// -------------------------------------------------------------------- colors

#[test]
fn color_hex_forms() {
    assert_eq!(parse_color("#fff"), Some(Rgba::new(255, 255, 255, 255)));
    assert_eq!(parse_color("#ff8800"), Some(Rgba::new(255, 136, 0, 255)));
    assert_eq!(parse_color("#11223344"), Some(Rgba::new(17, 34, 51, 68)));
    assert_eq!(parse_color("#ff"), None);
    assert_eq!(parse_color("#gg0000"), None);
}

#[test]
fn color_rgb_forms() {
    assert_eq!(parse_color("rgb(1,2,3)"), Some(Rgba::new(1, 2, 3, 255)));
    assert_eq!(
        parse_color("rgba(1,2,3,0.5)"),
        Some(Rgba::new(1, 2, 3, 128))
    );
    assert_eq!(
        parse_color("rgba(1,2,3/0.5)"),
        Some(Rgba::new(1, 2, 3, 128))
    );
    assert_eq!(parse_color("rgb(300,0,0)"), Some(Rgba::new(255, 0, 0, 255)));
    assert_eq!(parse_color("rgb(1,2)"), None);
    assert_eq!(parse_color("rgba(1,2,3,abc)"), None);
    assert_eq!(parse_color("rgb(1,2,3"), None);
}

#[test]
fn color_named_and_invalid() {
    assert_eq!(parse_color("black"), Some(Rgba::BLACK));
    assert_eq!(parse_color("white"), Some(Rgba::new(255, 255, 255, 255)));
    assert_eq!(parse_color("red"), Some(Rgba::new(255, 0, 0, 255)));
    assert_eq!(parse_color("green"), Some(Rgba::new(0, 128, 0, 255)));
    assert_eq!(parse_color("blue"), Some(Rgba::new(0, 0, 255, 255)));
    assert_eq!(parse_color("gray"), Some(Rgba::new(128, 128, 128, 255)));
    assert_eq!(parse_color("grey"), Some(Rgba::new(128, 128, 128, 255)));
    assert_eq!(parse_color("transparent"), Some(Rgba::new(0, 0, 0, 0)));
    assert_eq!(parse_color("chartreuse"), None);
}

// ------------------------------------------------------------- display + box

#[test]
fn display_maps_all() {
    let cases = [
        ("block", Display::Block),
        ("inline", Display::Inline),
        ("inline-block", Display::InlineBlock),
        ("flex", Display::Flex),
        ("none", Display::None),
        ("table", Display::Table),
        ("table-row", Display::TableRow),
        ("table-cell", Display::TableCell),
        ("table-header-group", Display::TableHeaderGroup),
        ("table-footer-group", Display::TableFooterGroup),
        ("list-item", Display::ListItem),
        ("weird", Display::Block),
    ];
    for (css, want) in cases {
        assert_eq!(resolve(&[("display", css)]).display, want);
    }
    assert_eq!(resolve(&[]).display, Display::Block);
}

#[test]
fn box_sizing_and_position() {
    assert_eq!(
        resolve(&[("box-sizing", "border-box")]).box_sizing,
        BoxSizing::BorderBox
    );
    assert_eq!(
        resolve(&[("box-sizing", "content-box")]).box_sizing,
        BoxSizing::ContentBox
    );
    assert_eq!(
        resolve(&[("position", "relative")]).position,
        Position::Relative
    );
    assert_eq!(
        resolve(&[("position", "static")]).position,
        Position::Static
    );
    assert_eq!(
        resolve(&[("position", "absolute")]).position,
        Position::Absolute
    );
    // Inset offsets + z-index parse into the box model.
    let s = resolve(&[
        ("position", "absolute"),
        ("top", "10px"),
        ("left", "20px"),
        ("z-index", "5"),
    ]);
    assert_eq!(s.inset_top, LengthPct::Px(10.0));
    assert_eq!(s.inset_left, LengthPct::Px(20.0));
    assert_eq!(s.z_index, Some(5));
    assert!(s.position.is_out_of_flow());
}

#[test]
fn margin_shorthand_arity() {
    approx(resolve(&[("margin", "5px")]).margin.top, 5.0);
    let two = resolve(&[("margin", "1px 2px")]).margin;
    assert_eq!(
        (two.top, two.right, two.bottom, two.left),
        (1.0, 2.0, 1.0, 2.0)
    );
    let three = resolve(&[("margin", "1px 2px 3px")]).margin;
    assert_eq!(
        (three.top, three.right, three.bottom, three.left),
        (1.0, 2.0, 3.0, 2.0)
    );
    let four = resolve(&[("margin", "1px 2px 3px 4px")]).margin;
    assert_eq!(
        (four.top, four.right, four.bottom, four.left),
        (1.0, 2.0, 3.0, 4.0)
    );
    assert_eq!(
        resolve(&[("margin", "1px 2px 3px 4px 5px")]).margin,
        Edges::default()
    );
}

#[test]
fn margin_longhand_overrides_shorthand() {
    let m = resolve(&[("margin", "5px"), ("margin-left", "9px")]).margin;
    assert_eq!((m.top, m.left), (5.0, 9.0));
}

#[test]
fn padding_percent_against_cb_width() {
    // 10% of cb_width 200 = 20
    approx(resolve(&[("padding", "10%")]).padding.top, 20.0);
}

#[test]
fn borders_shorthand_side_and_longhand() {
    let b = resolve(&[
        ("border", "2px solid red"),
        ("border-top", "4px solid blue"),
        ("border-left-width", "3px"),
        ("border-left-color", "green"),
    ])
    .border;
    assert_eq!(b.right.width, 2);
    assert_eq!(b.right.color, Some(Rgba::new(255, 0, 0, 255)));
    assert_eq!(b.top.width, 4);
    assert_eq!(b.top.color, Some(Rgba::new(0, 0, 255, 255)));
    assert_eq!(b.left.width, 3);
    assert_eq!(b.left.color, Some(Rgba::new(0, 128, 0, 255)));
    approx(b.widths().top, 4.0);
}

#[test]
fn all_sides_border_color_longhand_overrides_shorthand() {
    // `border:1px solid transparent` + `border-color:#72777d` (Codex radio icon):
    // the all-sides `border-color` longhand must override the shorthand's colour, or
    // the circle outline is invisible.
    let b = resolve(&[
        ("border", "1px solid transparent"),
        ("border-color", "#72777d"),
    ])
    .border;
    assert_eq!(b.top.color, Some(Rgba::new(0x72, 0x77, 0x7d, 255)));
    assert_eq!(b.left.color, Some(Rgba::new(0x72, 0x77, 0x7d, 255)));
    assert_eq!(b.top.width, 1);
}

#[test]
fn border_keyword_widths() {
    assert_eq!(resolve(&[("border", "thin")]).border.top.width, 1);
    assert_eq!(resolve(&[("border", "medium")]).border.top.width, 3);
    assert_eq!(resolve(&[("border", "thick")]).border.top.width, 5);
    assert_eq!(resolve(&[("border", "none")]).border.top.width, 0);
    assert_eq!(resolve(&[]).border, BorderEdges::default());
}

#[test]
fn width_height_min_max() {
    let s = resolve(&[
        ("width", "100px"),
        ("height", "50%"),
        ("min-width", "10px"),
        ("max-width", "auto"),
        ("min-height", "garbage"),
    ]);
    assert_eq!(s.width, LengthPct::Px(100.0));
    assert_eq!(s.height, LengthPct::Pct(50.0));
    assert_eq!(s.min_width, LengthPct::Px(10.0));
    assert_eq!(s.max_width, LengthPct::Auto);
    assert_eq!(s.min_height, LengthPct::Px(0.0)); // garbage -> default
    assert_eq!(resolve(&[]).width, LengthPct::Auto);
}

// ----------------------------------------------------------------- text/font

#[test]
fn font_size_resolution() {
    approx(resolve(&[("font-size", "32px")]).font_size, 32.0);
    approx(resolve(&[("font-size", "2em")]).font_size, 32.0);
    approx(resolve(&[("font-size", "150%")]).font_size, 24.0);
    approx(resolve(&[("font-size", "abc")]).font_size, 16.0);
    approx(resolve(&[]).font_size, 16.0);
}

#[test]
fn font_families_split() {
    assert_eq!(
        resolve(&[("font-family", "'Go', \"Arial\", ,Helvetica")]).font_families,
        vec![
            "Go".to_string(),
            "Arial".to_string(),
            "Helvetica".to_string()
        ]
    );
    assert!(resolve(&[]).font_families.is_empty());
}

#[test]
fn font_weight_and_style() {
    assert_eq!(resolve(&[("font-weight", "normal")]).font_weight, 400);
    assert_eq!(resolve(&[("font-weight", "bold")]).font_weight, 700);
    assert_eq!(resolve(&[("font-weight", "600")]).font_weight, 600);
    assert_eq!(resolve(&[("font-weight", "xyz")]).font_weight, 400);
    assert!(resolve(&[("font-style", "italic")]).italic);
    assert!(resolve(&[("font-style", "oblique")]).italic);
    assert!(!resolve(&[("font-style", "normal")]).italic);
    assert!(!resolve(&[]).italic);
}

#[test]
fn color_resolution() {
    assert_eq!(
        resolve(&[("color", "red")]).color,
        Rgba::new(255, 0, 0, 255)
    );
    assert_eq!(resolve(&[("color", "nope")]).color, Rgba::BLACK);
    assert_eq!(resolve(&[]).color, Rgba::BLACK);
}

#[test]
fn line_height_forms() {
    assert_eq!(resolve(&[]).line_height, None);
    assert_eq!(resolve(&[("line-height", "normal")]).line_height, None);
    approx(
        resolve(&[("line-height", "1.5")]).line_height.unwrap(),
        24.0,
    );
    approx(
        resolve(&[("line-height", "20px")]).line_height.unwrap(),
        20.0,
    );
    assert_eq!(resolve(&[("line-height", "abc")]).line_height, None);
}

#[test]
fn align_white_space_valign() {
    assert_eq!(resolve(&[("text-align", "right")]).text_align, Align::Right);
    assert_eq!(
        resolve(&[("text-align", "center")]).text_align,
        Align::Center
    );
    assert_eq!(
        resolve(&[("text-align", "justify")]).text_align,
        Align::Justify
    );
    assert_eq!(resolve(&[("text-align", "left")]).text_align, Align::Left);
    assert_eq!(
        resolve(&[("white-space", "pre")]).white_space,
        WhiteSpace::Pre
    );
    assert_eq!(
        resolve(&[("white-space", "nowrap")]).white_space,
        WhiteSpace::NoWrap
    );
    assert_eq!(
        resolve(&[("white-space", "normal")]).white_space,
        WhiteSpace::Normal
    );
    let valigns = [
        ("sub", VAlign::Sub),
        ("super", VAlign::Super),
        ("middle", VAlign::Middle),
        ("top", VAlign::Top),
        ("bottom", VAlign::Bottom),
        ("baseline", VAlign::Baseline),
    ];
    for (css, want) in valigns {
        assert_eq!(resolve(&[("vertical-align", css)]).vertical_align, want);
    }
}

#[test]
fn letter_spacing_resolves() {
    approx(resolve(&[("letter-spacing", "2px")]).letter_spacing, 2.0);
    approx(resolve(&[]).letter_spacing, 0.0);
}

#[test]
fn breaks_orphans_widows_background() {
    assert_eq!(
        resolve(&[("break-before", "avoid")]).break_before,
        BreakRule::Avoid
    );
    assert_eq!(
        resolve(&[("break-before", "page")]).break_before,
        BreakRule::Page
    );
    assert_eq!(
        resolve(&[("break-after", "column")]).break_after,
        BreakRule::Page
    );
    assert_eq!(
        resolve(&[("break-after", "auto")]).break_after,
        BreakRule::Auto
    );
    assert!(resolve(&[("break-inside", "avoid")]).break_inside_avoid);
    assert!(!resolve(&[]).break_inside_avoid);
    assert_eq!(resolve(&[("orphans", "3")]).orphans, 3);
    assert_eq!(resolve(&[("widows", "bad")]).widows, 2);
    assert_eq!(resolve(&[]).orphans, 2);
    assert_eq!(
        resolve(&[("background-color", "blue")]).background,
        Some(Rgba::new(0, 0, 255, 255))
    );
    assert_eq!(resolve(&[]).background, None);
}

#[test]
fn from_pairs_get_roundtrip() {
    let s = ComputedStyle::from_pairs([("color", "red"), ("display", "flex")]);
    assert_eq!(s.get("color"), Some("red"));
    assert_eq!(s.get("missing"), None);
}

// ---------------------------------------------------------------- background shorthand

#[test]
fn background_shorthand_resolves_color() {
    // The `background` shorthand (not just the `background-color` longhand) must
    // set the box's background — real stylesheets use the shorthand pervasively.
    let green = Rgba {
        r: 0,
        g: 255,
        b: 0,
        a: 255,
    };
    assert_eq!(
        resolve(&[("background-color", "#00ff00")]).background,
        Some(green)
    );
    assert_eq!(
        resolve(&[("background", "#00ff00")]).background,
        Some(green)
    );
    // A full shorthand with an image + keywords still yields the colour token.
    assert_eq!(
        resolve(&[("background", "#00ff00 url(x.png) no-repeat center")]).background,
        Some(green)
    );
    // rgb() functional colour inside the shorthand (kept whole despite spaces).
    assert_eq!(
        resolve(&[("background", "rgb(0, 255, 0) url(x.png)")]).background,
        Some(green)
    );
    // The longhand still wins when both are present.
    assert_eq!(
        resolve(&[("background", "#ff0000"), ("background-color", "#00ff00")]).background,
        Some(green)
    );
    // No colour token → no background (an image-only shorthand).
    assert_eq!(
        resolve(&[("background", "url(x.png) no-repeat")]).background,
        None
    );
}

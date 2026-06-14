//! Document-domain filter + function tests (§2.12, §3.3). Covers every branch
//! of currency/number/percent/ordinal/pad/truncate/wordwrap/date/datetime/now.

use serde_json::{json, Value as Json};
use turbo_pdf_core::{compile, CompileOptions, ErrorCode};

fn render(tpl: &str, data: &Json) -> String {
    let (program, _) = compile(tpl, &CompileOptions::default()).expect("compile");
    program.render_markup(data, None).expect("render").0
}

fn render_now(tpl: &str, now: i64) -> String {
    let (program, _) = compile(tpl, &CompileOptions::default()).expect("compile");
    program
        .render_markup(&json!({}), Some(now))
        .expect("render")
        .0
}

// --------------------------------------------------------------------------
// currency
// --------------------------------------------------------------------------

#[test]
fn currency_anglo_default_locale() {
    assert_eq!(
        render(r#"{{ 1234.5 | currency("USD") }}"#, &json!({})),
        "$1,234.50"
    );
}

#[test]
fn ac_2_24_currency_euro_locale() {
    assert_eq!(
        render(r#"{{ 1234.5 | currency("EUR", "pt-PT") }}"#, &json!({})),
        "1.234,50\u{00a0}€"
    );
}

#[test]
fn currency_negative_and_symbols() {
    assert_eq!(
        render(r#"{{ -5 | currency("GBP") }}"#, &json!({})),
        "-£5.00"
    );
    assert_eq!(
        render(r#"{{ 10 | currency("JPY") }}"#, &json!({})),
        "¥10.00"
    );
    assert_eq!(
        render(r#"{{ 10 | currency("PLN", "pl") }}"#, &json!({})),
        "10,00\u{00a0}zł"
    );
}

#[test]
fn currency_unknown_code_uses_code_prefix() {
    assert_eq!(
        render(r#"{{ 1 | currency("xyz") }}"#, &json!({})),
        "XYZ 1.00"
    );
}

// --------------------------------------------------------------------------
// number / percent / ordinal
// --------------------------------------------------------------------------

#[test]
fn number_default_and_explicit_decimals() {
    assert_eq!(render("{{ 1000 | number }}", &json!({})), "1,000");
    assert_eq!(render("{{ 1000.25 | number }}", &json!({})), "1,000.25");
    assert_eq!(render("{{ 1000 | number(2) }}", &json!({})), "1,000.00");
}

#[test]
fn number_negative_keeps_sign() {
    assert_eq!(render("{{ -5 | number }}", &json!({})), "-5");
}

#[test]
fn percent_default_and_decimals() {
    assert_eq!(render("{{ 0.25 | percent }}", &json!({})), "25%");
    assert_eq!(render("{{ 0.125 | percent(1) }}", &json!({})), "12.5%");
}

#[test]
fn ordinal_suffixes() {
    let tpl = "{{ n | ordinal }}";
    assert_eq!(render(tpl, &json!({"n": 1})), "1st");
    assert_eq!(render(tpl, &json!({"n": 2})), "2nd");
    assert_eq!(render(tpl, &json!({"n": 3})), "3rd");
    assert_eq!(render(tpl, &json!({"n": 4})), "4th");
    assert_eq!(render(tpl, &json!({"n": 11})), "11th");
    assert_eq!(render(tpl, &json!({"n": 22})), "22nd");
}

// --------------------------------------------------------------------------
// pad / truncate / wordwrap
// --------------------------------------------------------------------------

#[test]
fn pad_default_and_custom_fill() {
    assert_eq!(render("{{ 7 | pad(3) }}", &json!({})), "  7");
    assert_eq!(render(r#"{{ 7 | pad(3, "0") }}"#, &json!({})), "007");
    assert_eq!(render("{{ 12345 | pad(3) }}", &json!({})), "12345");
}

#[test]
fn truncate_variants() {
    assert_eq!(render("{{ 'hi' | truncate(5) }}", &json!({})), "hi");
    assert_eq!(
        render("{{ 'hello world' | truncate(5) }}", &json!({})),
        "hell…"
    );
    assert_eq!(
        render(r#"{{ 'hello world' | truncate(6, "..") }}"#, &json!({})),
        "hell.."
    );
}

#[test]
fn wordwrap_wraps_and_handles_empty() {
    assert_eq!(
        render("{{ 'aa bb cc' | wordwrap(5) }}", &json!({})),
        "aa bb\ncc"
    );
    assert_eq!(render("{{ '' | wordwrap(5) }}", &json!({})), "");
}

// --------------------------------------------------------------------------
// date / datetime / now
// --------------------------------------------------------------------------

#[test]
fn date_from_timestamp_default_fmt() {
    assert_eq!(render("{{ 0 | date }}", &json!({})), "1970-01-01");
}

#[test]
fn date_from_rfc3339_with_offset_tz() {
    let tpl = r#"{{ ts | datetime("YYYY-MM-DD HH:mm:ss", "+02:00") }}"#;
    assert_eq!(
        render(tpl, &json!({"ts": "2020-06-14T10:00:00Z"})),
        "2020-06-14 12:00:00"
    );
}

#[test]
fn date_from_plain_date_string() {
    // '.' is not HTML-escaped; '/' would be, so use a dot format here.
    assert_eq!(
        render(r#"{{ "2020-06-14" | date("DD.MM.YY") }}"#, &json!({})),
        "14.06.20"
    );
}

#[test]
fn datetime_default_fmt_and_utc_tz() {
    assert_eq!(
        render("{{ 0 | datetime }}", &json!({})),
        "1970-01-01 00:00:00"
    );
    let tpl = r#"{{ 0 | datetime("YYYY-MM-DD HH:mm:ss", "UTC") }}"#;
    assert_eq!(render(tpl, &json!({})), "1970-01-01 00:00:00");
    assert_eq!(
        render(r#"{{ 0 | datetime("HH:mm", "Z") }}"#, &json!({})),
        "00:00"
    );
}

#[test]
fn date_negative_offset_tz() {
    let tpl = r#"{{ 0 | datetime("HH:mm", "-05:30") }}"#;
    assert_eq!(render(tpl, &json!({})), "18:30");
}

#[test]
fn date_bad_string_errors() {
    let (program, _) = compile(r#"{{ "not-a-date" | date }}"#, &CompileOptions::default()).unwrap();
    assert_eq!(
        program.render_markup(&json!({}), None).unwrap_err().code,
        ErrorCode::Render
    );
}

#[test]
fn date_non_timestamp_value_errors() {
    let (program, _) = compile("{{ x | date }}", &CompileOptions::default()).unwrap();
    let err = program.render_markup(&json!({"x": 1.5}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::Render);
}

#[test]
fn date_bad_tz_no_sign_errors() {
    let (program, _) = compile(
        r#"{{ 0 | date("YYYY", "0200") }}"#,
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(
        program.render_markup(&json!({}), None).unwrap_err().code,
        ErrorCode::Render
    );
}

#[test]
fn date_bad_tz_no_colon_errors() {
    let (program, _) = compile(
        r#"{{ 0 | date("YYYY", "+0200") }}"#,
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(
        program.render_markup(&json!({}), None).unwrap_err().code,
        ErrorCode::Render
    );
}

#[test]
fn date_tz_out_of_range_errors() {
    let (program, _) = compile(
        r#"{{ 0 | date("YYYY", "+99:00") }}"#,
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(
        program.render_markup(&json!({}), None).unwrap_err().code,
        ErrorCode::Render
    );
}

#[test]
fn date_bad_tz_non_numeric_errors() {
    let (program, _) = compile(
        r#"{{ 0 | date("YYYY", "+ab:cd") }}"#,
        &CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(
        program.render_markup(&json!({}), None).unwrap_err().code,
        ErrorCode::Render
    );
}

#[test]
fn date_timestamp_out_of_range_errors() {
    let (program, _) = compile("{{ x | date }}", &CompileOptions::default()).unwrap();
    let err = program
        .render_markup(&json!({"x": 999999999999999999i64}), None)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Render);
}

#[test]
fn ac_3_6e_now_pinned_for_determinism() {
    assert_eq!(
        render_now(r#"{{ date(now(), "YYYY-MM-DD") }}"#, 0),
        "1970-01-01"
    );
}

#[test]
fn now_unset_is_render_error() {
    let (program, _) = compile("{{ now() }}", &CompileOptions::default()).unwrap();
    assert_eq!(
        program.render_markup(&json!({}), None).unwrap_err().code,
        ErrorCode::Render
    );
}

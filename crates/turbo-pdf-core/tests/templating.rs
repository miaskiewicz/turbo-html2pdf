//! Templating-layer acceptance tests (§2). Each test names the AC it covers.

use std::collections::HashMap;

use serde_json::{json, Value as Json};
use turbo_pdf_core::{compile, CompileOptions, ErrorCode, MissingPolicy};

/// Compile `tpl` with default options and render against `data`, returning markup.
fn render(tpl: &str, data: &Json) -> String {
    let (program, diags) = compile(tpl, &CompileOptions::default()).expect("compile");
    assert!(diags.is_empty());
    let (markup, rdiags) = program.render_markup(data, None).expect("render");
    assert!(rdiags.is_empty());
    markup
}

/// Render with no data.
fn render_nil(tpl: &str) -> String {
    render(tpl, &json!({}))
}

// --------------------------------------------------------------------------
// §2.0 — pure-Jinja templates render through the evaluator
// --------------------------------------------------------------------------

#[test]
fn ac_2_0_pure_jinja_renders() {
    assert_eq!(
        render("Hello {{ name }}", &json!({"name": "world"})),
        "Hello world"
    );
}

#[test]
fn ac_2_0b_program_renders_many_times() {
    let (program, _) = compile("{{ n }}", &CompileOptions::default()).unwrap();
    assert_eq!(
        program.render_markup(&json!({"n": 1}), None).unwrap().0,
        "1"
    );
    assert_eq!(
        program.render_markup(&json!({"n": 2}), None).unwrap().0,
        "2"
    );
}

// --------------------------------------------------------------------------
// §2.2 — expression language
// --------------------------------------------------------------------------

#[test]
fn ac_2_1_infix_precedence() {
    assert_eq!(
        render_nil("{% if 2 + 3 * 4 == 14 %}Y{% else %}N{% endif %}"),
        "Y"
    );
    let data = json!({"invoice": {"total": 2000}, "customer": {"tier": "pro"}});
    let tpl = r#"{% if invoice.total > 1000 and customer.tier == "pro" %}OK{% endif %}"#;
    assert_eq!(render(tpl, &data), "OK");
}

#[test]
fn ac_2_2_path_access_left_to_right() {
    let data = json!({"a": {"b": [{"d": "X"}]}, "c": 0});
    assert_eq!(render("{{ a.b[c].d }}", &data), "X");
    assert_eq!(render(r#"{{ a["b"][0]["d"] }}"#, &data), "X");
}

#[test]
fn ac_2_3_filters_chainable() {
    assert_eq!(render_nil("{{ 'ab' | upper | length }}"), "2");
}

#[test]
fn ac_2_4_truthiness_matches_jinja() {
    assert_eq!(
        render(
            r#"{% if items %}T{% else %}F{% endif %}"#,
            &json!({"items": []})
        ),
        "F"
    );
    assert_eq!(
        render(
            r#"{% if items %}T{% else %}F{% endif %}"#,
            &json!({"items": [0]})
        ),
        "T"
    );
    assert_eq!(
        render(r#"{% if s %}T{% else %}F{% endif %}"#, &json!({"s": "0"})),
        "T"
    );
}

// --------------------------------------------------------------------------
// §2.4 — interpolation + escaping
// --------------------------------------------------------------------------

#[test]
fn ac_2_5_autoescape_on() {
    assert_eq!(render_nil("{{ '<b>' }}"), "&lt;b&gt;");
}

#[test]
fn ac_2_6_safe_filter_unescaped() {
    assert_eq!(render_nil("{{ '<b>x</b>' | safe }}"), "<b>x</b>");
}

// --------------------------------------------------------------------------
// §2.5–2.7 — control flow
// --------------------------------------------------------------------------

#[test]
fn ac_2_8_conditionals_one_branch() {
    let tpl = r#"{% if s == "paid" %}P{% elif s == "overdue" %}O{% else %}D{% endif %}"#;
    assert_eq!(render(tpl, &json!({"s": "overdue"})), "O");
    assert_eq!(render(tpl, &json!({"s": "x"})), "D");
}

#[test]
fn ac_2_9_membership_in() {
    let tpl = r#"{% if tier in ["pro", "plus"] %}Y{% else %}N{% endif %}"#;
    assert_eq!(render(tpl, &json!({"tier": "plus"})), "Y");
    assert_eq!(render(tpl, &json!({"tier": "free"})), "N");
}

#[test]
fn ac_2_11_loop_object() {
    let tpl = "{% for x in xs %}{{ loop.index }}:{{ loop.first }}:{{ loop.last }}:{{ loop.length }};{% endfor %}";
    assert_eq!(render(tpl, &json!({"xs": ["a"]})), "1:true:true:1;");
}

#[test]
fn ac_2_12_for_else_empty() {
    let tpl = "{% for x in xs %}{{ x }}{% else %}EMPTY{% endfor %}";
    assert_eq!(render(tpl, &json!({"xs": []})), "EMPTY");
}

#[test]
fn ac_2_13_nested_loops_distinct_state() {
    let tpl = "{% for i in a %}{% for j in b %}{{ loop.index }}{% endfor %}-{{ loop.index }};{% endfor %}";
    assert_eq!(
        render(tpl, &json!({"a": [1, 2], "b": [1, 1]})),
        "12-1;12-2;"
    );
}

#[test]
fn ac_2_10_dict_iteration_is_insertion_order() {
    let data = json!({"d": {"z": 1, "a": 2, "m": 3}});
    assert_eq!(
        render("{% for k, v in d | items %}{{ k }}{% endfor %}", &data),
        "zam"
    );
}

// --------------------------------------------------------------------------
// §2.8 — partials / macros / includes + recursion cap
// --------------------------------------------------------------------------

fn opts_with_partials(pairs: &[(&str, &str)]) -> CompileOptions {
    let mut partials = HashMap::new();
    for (k, v) in pairs {
        partials.insert((*k).to_string(), (*v).to_string());
    }
    CompileOptions {
        partials,
        ..Default::default()
    }
}

#[test]
fn ac_2_14_include_partial() {
    let opts = opts_with_partials(&[("addr", "ADDR:{{ city }}")]);
    let (program, _) = compile("{% include 'addr' %}", &opts).unwrap();
    assert_eq!(
        program
            .render_markup(&json!({"city": "Kraków"}), None)
            .unwrap()
            .0,
        "ADDR:Kraków"
    );
}

#[test]
fn ac_2_15_macro_local_scope() {
    let opts = opts_with_partials(&[("m", "{% macro hi(name) %}Hi {{ name }}{% endmacro %}")]);
    let tpl = "{% import 'm' as m %}{{ m.hi('Ola') }}";
    let (program, _) = compile(tpl, &opts).unwrap();
    assert_eq!(program.render_markup(&json!({}), None).unwrap().0, "Hi Ola");
}

#[test]
fn ac_2_17_include_recursion_depth_capped() {
    let opts = CompileOptions {
        partials: HashMap::from([("loop".to_string(), "{% include 'loop' %}".to_string())]),
        include_max_depth: 4,
        ..Default::default()
    };
    let (program, _) = compile("{% include 'loop' %}", &opts).unwrap();
    let err = program.render_markup(&json!({}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::IncludeDepthExceeded);
}

// --------------------------------------------------------------------------
// §2.9 — undefined / missing policy
// --------------------------------------------------------------------------

#[test]
fn ac_2_18_strict_undefined_errors() {
    let (program, _) = compile("{{ missing }}", &CompileOptions::default()).unwrap();
    let err = program.render_markup(&json!({}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::UndefinedValue);
    assert!(err.span.line >= 1);
}

#[test]
fn ac_2_19_empty_policy_renders_blank() {
    let opts = CompileOptions {
        missing_policy: MissingPolicy::Empty,
        ..Default::default()
    };
    let (program, _) = compile("[{{ missing }}]", &opts).unwrap();
    assert_eq!(program.render_markup(&json!({}), None).unwrap().0, "[]");
}

#[test]
fn ac_2_21_default_filter_fallback() {
    assert_eq!(render(r#"{{ x | default("—") }}"#, &json!({})), "—");
    assert_eq!(render(r#"{{ x | default("—") }}"#, &json!({"x": "v"})), "v");
}

// --------------------------------------------------------------------------
// §2.10–2.11 — whitespace control + comments
// --------------------------------------------------------------------------

#[test]
fn ac_2_22_trim_blocks_default_on() {
    // trim_blocks removes the newline after a block tag; lstrip_blocks strips
    // leading whitespace before one. Result is clean, gap-free output.
    assert_eq!(render_nil("    {% if true %}\nX\n{% endif %}\nY"), "X\nY");
}

#[test]
fn ac_2_23_comments_stripped() {
    assert_eq!(render_nil("a{# hidden #}b"), "ab");
}

// --------------------------------------------------------------------------
// §9.1 — compile errors carry codes + spans
// --------------------------------------------------------------------------

#[test]
fn ac_9_1_syntax_error_has_span() {
    let err = compile("{% if %}", &CompileOptions::default()).unwrap_err();
    assert_eq!(err.code, ErrorCode::TemplateSyntax);
    assert!(err.span.line >= 1);
}

#[test]
fn ac_9_1_bad_partial_is_compile_error() {
    let opts = opts_with_partials(&[("bad", "{% endif %}")]);
    let err = compile("ok", &opts).unwrap_err();
    assert_eq!(err.code, ErrorCode::TemplateSyntax);
}

#[test]
fn unknown_filter_is_typed_error() {
    let (program, _) = compile("{{ x | nope }}", &CompileOptions::default()).unwrap();
    let err = program.render_markup(&json!({"x": 1}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownFilter);
}

#[test]
fn unknown_function_is_typed_error() {
    let (program, _) = compile("{{ nope() }}", &CompileOptions::default()).unwrap();
    let err = program.render_markup(&json!({}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownFilter);
}

#[test]
fn unknown_test_is_typed_error() {
    let (program, _) = compile(
        "{% if 1 is bogus %}x{% endif %}",
        &CompileOptions::default(),
    )
    .unwrap();
    let err = program.render_markup(&json!({}), None).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnknownFilter);
}

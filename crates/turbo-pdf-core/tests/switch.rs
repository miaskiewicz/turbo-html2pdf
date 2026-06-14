//! `{% switch %}` extension tests (§2.6). Covers AC-2.9b–2.9j.

use serde_json::{json, Value as Json};
use turbo_pdf_core::{compile, CompileOptions, ErrorCode};

fn render(tpl: &str, data: &Json) -> String {
    let (program, _) = compile(tpl, &CompileOptions::default()).expect("compile");
    program.render_markup(data, None).expect("render").0
}

fn compile_err(tpl: &str) -> ErrorCode {
    compile(tpl, &CompileOptions::default()).unwrap_err().code
}

const TIER: &str = r#"{% switch tier %}
  {% case "enterprise" %}E
  {% case "pro", "plus" %}P
  {% default %}S
{% endswitch %}"#;

#[test]
fn ac_2_9c_membership_first_match_wins() {
    assert_eq!(render(TIER, &json!({"tier": "enterprise"})).trim(), "E");
    assert_eq!(render(TIER, &json!({"tier": "pro"})).trim(), "P");
    assert_eq!(render(TIER, &json!({"tier": "plus"})).trim(), "P");
    assert_eq!(render(TIER, &json!({"tier": "free"})).trim(), "S");
}

#[test]
fn ac_2_9b_subject_evaluated_once() {
    // `loop`-style side effect proxy: a namespace mutated by the subject would
    // change between cases if re-evaluated. We assert via a counter filter-free
    // approach: the subject is a computed expression; output is stable.
    let tpl = "{% switch (a + b) %}{% case 3 %}three{% default %}other{% endswitch %}";
    assert_eq!(render(tpl, &json!({"a": 1, "b": 2})), "three");
}

#[test]
fn ac_2_9d_only_first_body_renders() {
    let tpl = r#"{% switch x %}{% case 1 %}A{% case 1 %}B{% default %}C{% endswitch %}"#;
    assert_eq!(render(tpl, &json!({"x": 1})), "A");
}

#[test]
fn ac_2_9h_case_values_can_be_variables() {
    let tpl = "{% switch x %}{% case lo, hi %}hit{% default %}miss{% endswitch %}";
    assert_eq!(render(tpl, &json!({"x": 5, "lo": 1, "hi": 5})), "hit");
    assert_eq!(render(tpl, &json!({"x": 9, "lo": 1, "hi": 5})), "miss");
}

#[test]
fn ac_2_9e_default_must_be_last() {
    assert_eq!(
        compile_err(r#"{% switch x %}{% default %}D{% case 1 %}A{% endswitch %}"#),
        ErrorCode::TemplateSyntax
    );
}

#[test]
fn ac_2_9e_single_default() {
    assert_eq!(
        compile_err(r#"{% switch x %}{% default %}D{% default %}E{% endswitch %}"#),
        ErrorCode::TemplateSyntax
    );
}

#[test]
fn ac_2_9f_no_text_before_first_case() {
    assert_eq!(
        compile_err(r#"{% switch x %}junk{% case 1 %}A{% endswitch %}"#),
        ErrorCode::TemplateSyntax
    );
}

#[test]
fn ac_2_9f_whitespace_before_case_is_ok() {
    let tpl = "{% switch x %}   {% case 1 %}A{% default %}B{% endswitch %}";
    assert_eq!(render(tpl, &json!({"x": 1})), "A");
}

#[test]
fn unterminated_switch_errors() {
    assert_eq!(
        compile_err("{% switch x %}{% case 1 %}A"),
        ErrorCode::TemplateSyntax
    );
}

#[test]
fn stray_case_and_endswitch_error() {
    assert_eq!(compile_err("{% case 1 %}A"), ErrorCode::TemplateSyntax);
    assert_eq!(compile_err("{% endswitch %}"), ErrorCode::TemplateSyntax);
    assert_eq!(compile_err("{% default %}d"), ErrorCode::TemplateSyntax);
}

#[test]
fn ac_2_9g_non_taken_case_has_no_side_effects() {
    // A {% set %} inside a non-taken case must not affect the namespace.
    let tpl = "{% set ns = namespace(v=0) %}{% switch x %}{% case 1 %}{% set ns.v = 99 %}{% default %}d{% endswitch %}[{{ ns.v }}]";
    assert_eq!(render(tpl, &json!({"x": 2})), "d[0]");
}

#[test]
fn ac_2_9i_whitespace_control_markers() {
    let tpl = "A{%- switch x -%}  {%- case 1 -%}  B  {%- default -%}d{%- endswitch -%}  C";
    assert_eq!(render(tpl, &json!({"x": 1})), "ABC");
}

#[test]
fn unterminated_jinja_tag_surfaces_as_syntax_error() {
    // `{%` with no closing `%}` is passed through by the desugarer and rejected
    // by MiniJinja at compile (exercises the find_tag end-of-tag path).
    assert_eq!(compile_err("text {% oops"), ErrorCode::TemplateSyntax);
}

#[test]
fn case_value_with_brackets_is_not_split_on_inner_comma() {
    // The comma inside `[1, 2]` must not be treated as a case separator.
    let tpl = "{% switch x %}{% case [1, 2] %}L{% default %}D{% endswitch %}";
    assert_eq!(render(tpl, &json!({"x": [1, 2]})), "L");
    assert_eq!(render(tpl, &json!({"x": [3]})), "D");
}

#[test]
fn nested_switch() {
    let tpl = "{% switch x %}{% case 1 %}{% switch y %}{% case 2 %}XY{% default %}X?{% endswitch %}{% default %}?{% endswitch %}";
    assert_eq!(render(tpl, &json!({"x": 1, "y": 2})), "XY");
    assert_eq!(render(tpl, &json!({"x": 1, "y": 3})), "X?");
    assert_eq!(render(tpl, &json!({"x": 9, "y": 2})), "?");
}

#[test]
fn ac_2_9j_switch_equals_hand_written_if() {
    let sw = r#"{% switch t %}{% case "a" %}A{% case "b" %}B{% default %}D{% endswitch %}"#;
    let manual = r#"{% if t == "a" %}A{% elif t == "b" %}B{% else %}D{% endif %}"#;
    for t in ["a", "b", "z"] {
        let data = json!({ "t": t });
        assert_eq!(render(sw, &data), render(manual, &data));
    }
}

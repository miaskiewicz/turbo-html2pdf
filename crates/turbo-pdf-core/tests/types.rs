//! Unit tests for the public error / diagnostic / option types (§8.1, §9).

use turbo_pdf_core::{
    compile, CompileError, CompileOptions, Diagnostics, ErrorCode, Lint, LintCode, Program,
    RenderError, Span, DEFAULT_INCLUDE_DEPTH,
};

#[test]
fn span_from_range_and_none() {
    let with_range = Span::new(3, Some(10..20));
    assert_eq!(with_range.line, 3);
    assert_eq!(with_range.byte_offset, 10);
    assert_eq!(with_range.col, 0);

    let without = Span::new(0, None);
    assert_eq!(without.byte_offset, 0);
}

#[test]
fn diagnostics_push_and_is_empty() {
    let mut diags = Diagnostics::default();
    assert!(diags.is_empty());
    diags.push(
        LintCode::UnsupportedCss,
        "float ignored",
        Span::new(1, None),
    );
    assert!(!diags.is_empty());
    assert_eq!(diags.lints.len(), 1);
    assert_eq!(diags.lints[0].code, LintCode::UnsupportedCss);
    assert_eq!(diags.lints[0].message, "float ignored");
}

#[test]
fn lint_construct_and_equality() {
    let span = Span::new(2, None);
    let a = Lint {
        code: LintCode::NotdefGlyph,
        message: "x".into(),
        span,
    };
    let b = Lint {
        code: LintCode::NotdefGlyph,
        message: "x".into(),
        span,
    };
    assert_eq!(a, b);
}

#[test]
fn compile_error_display_includes_line() {
    let err = CompileError {
        code: ErrorCode::TemplateSyntax,
        message: "boom".into(),
        span: Span::new(7, None),
    };
    assert_eq!(format!("{err}"), "TemplateSyntax at line 7: boom");
}

#[test]
fn render_error_display_includes_line() {
    let err = RenderError {
        code: ErrorCode::Render,
        message: "nope".into(),
        span: Span::new(4, None),
    };
    assert_eq!(format!("{err}"), "Render at line 4: nope");
}

#[test]
fn effective_depth_default_and_custom() {
    assert_eq!(
        CompileOptions::default().effective_depth(),
        DEFAULT_INCLUDE_DEPTH
    );
    let custom = CompileOptions {
        include_max_depth: 12,
        ..Default::default()
    };
    assert_eq!(custom.effective_depth(), 12);
}

#[test]
fn program_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Program>();
    // and actually usable: compile produces a Send+Sync handle.
    let (_program, _diags) = compile("ok", &CompileOptions::default()).unwrap();
}

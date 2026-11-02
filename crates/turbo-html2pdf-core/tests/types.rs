//! Unit tests for the public error / diagnostic / option types (§8.1, §9).
//!
//! Split at the S1 relocation: the layout-only cases moved to turbo-dom-layout's
//! `tests/types.rs`; the ones here need TKind directives, templating or the PDF
//! emitter.

use turbo_html2pdf_core::{compile, CompileOptions, Program, DEFAULT_INCLUDE_DEPTH};

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

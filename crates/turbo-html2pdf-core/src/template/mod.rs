//! The templating layer (§2): a MiniJinja-backed engine that parses + caches a
//! template at compile time and renders it against data, emitting intermediate
//! markup that never escapes to the caller (the §1 hard rule). Later phases
//! parse this markup into the node tree; for now it is the unit of test.

mod filters;
mod functions;
pub(crate) mod markup;
mod regions;
mod switch;

use std::collections::HashMap;

use minijinja::value::Value;
use minijinja::{AutoEscape, Environment};
use serde::Serialize;

use crate::error::{CompileError, Diagnostics, ErrorCode, RenderError, Span};
use crate::node::Node;
use crate::options::CompileOptions;

pub use functions::set_now;
pub use regions::{FOOTER, HEADER};

/// Internal name under which the caller's template is registered.
const MAIN: &str = "__main__";

/// A compiled, data-independent template program. `Send + Sync` so one program
/// renders concurrently across threads (§8.1, AC-8.1).
#[derive(Debug)]
pub struct Program {
    env: Environment<'static>,
    /// Whether the source carried a `<t:running-header>` / `<t:running-footer>`,
    /// so the orchestrator knows which region bands to reserve and render.
    has_header: bool,
    has_footer: bool,
}

fn configure(env: &mut Environment, opts: &CompileOptions) {
    env.set_undefined_behavior(opts.missing_policy.undefined_behavior());
    env.set_trim_blocks(true);
    env.set_lstrip_blocks(true);
    env.set_recursion_limit(opts.effective_depth() as usize);
    env.set_auto_escape_callback(|_name| AutoEscape::Html);
}

fn register(env: &mut Environment) {
    env.add_filter("currency", filters::currency);
    env.add_filter("number", filters::number);
    env.add_filter("percent", filters::percent);
    env.add_filter("ordinal", filters::ordinal);
    env.add_filter("pad", filters::pad);
    env.add_filter("truncate", filters::truncate);
    env.add_filter("wordwrap", filters::wordwrap);
    env.add_filter("date", filters::date);
    env.add_filter("datetime", filters::datetime);
    // date/datetime are also callable as functions for header field codes:
    // `{{ date(now(), "YYYY-MM-DD") }}` (§3.3, AC-3.6e).
    env.add_function("date", filters::date);
    env.add_function("datetime", filters::datetime);
    env.add_function("now", functions::now);
}

fn recursion_or_render(msg: &str) -> ErrorCode {
    if msg.contains("recursion") {
        ErrorCode::IncludeDepthExceeded
    } else {
        ErrorCode::Render
    }
}

fn code_of(kind: minijinja::ErrorKind, msg: &str) -> ErrorCode {
    use minijinja::ErrorKind as K;
    match kind {
        K::SyntaxError => ErrorCode::TemplateSyntax,
        K::UndefinedError => ErrorCode::UndefinedValue,
        K::UnknownFilter | K::UnknownFunction | K::UnknownMethod | K::UnknownTest => {
            ErrorCode::UnknownFilter
        }
        _ => recursion_or_render(msg),
    }
}

fn span_of(e: &minijinja::Error) -> Span {
    Span::new(e.line().unwrap_or(0) as u32, e.range())
}

fn map_compile_err(e: minijinja::Error) -> CompileError {
    let message = e.to_string();
    CompileError {
        code: code_of(e.kind(), &message),
        message,
        span: span_of(&e),
    }
}

fn map_render_err(e: minijinja::Error) -> RenderError {
    let message = e.to_string();
    RenderError {
        code: code_of(e.kind(), &message),
        message,
        span: span_of(&e),
    }
}

fn add_partials(
    env: &mut Environment,
    partials: &HashMap<String, String>,
) -> Result<(), CompileError> {
    for (name, source) in partials {
        let desugared = switch::desugar(source)?;
        env.add_template_owned(name.clone(), desugared)
            .map_err(map_compile_err)?;
    }
    Ok(())
}

/// Register one extracted region (header/footer) under `name`, if present.
fn add_region(
    env: &mut Environment,
    name: &'static str,
    source: Option<String>,
) -> Result<bool, CompileError> {
    let Some(src) = source else {
        return Ok(false);
    };
    let desugared = switch::desugar(&src)?;
    env.add_template_owned(name, desugared)
        .map_err(map_compile_err)?;
    Ok(true)
}

/// Compile a template into a reusable [`Program`] (§8.1). The MiniJinja template
/// and partials are parsed and cached here; per-render work is render-only.
///
/// Running header/footer regions (§3.0) are lifted out of the body source first
/// so they never render in body flow, and registered as their own templates the
/// orchestrator re-renders per page (§6.5–6.6).
pub fn compile(
    template: &str,
    opts: &CompileOptions,
) -> Result<(Program, Diagnostics), CompileError> {
    let mut env = Environment::new();
    configure(&mut env, opts);
    register(&mut env);
    add_partials(&mut env, &opts.partials)?;
    let (has_header, has_footer) = add_main_and_regions(&mut env, template)?;
    Ok((
        Program {
            env,
            has_header,
            has_footer,
        },
        Diagnostics::default(),
    ))
}

/// Lift the running regions out of `template`, register them plus the body's
/// `__main__` template on `env`, and report which regions were present.
fn add_main_and_regions(
    env: &mut Environment,
    template: &str,
) -> Result<(bool, bool), CompileError> {
    let extracted = regions::extract(template);
    let has_header = add_region(env, HEADER, extracted.header)?;
    let has_footer = add_region(env, FOOTER, extracted.footer)?;
    let desugared = switch::desugar(&extracted.body)?;
    // add_template_owned parses eagerly, so syntax errors surface here at compile.
    env.add_template_owned(MAIN, desugared)
        .map_err(map_compile_err)?;
    Ok((has_header, has_footer))
}

impl Program {
    /// Render the template to intermediate markup. `now` pins the `now()` clock
    /// for determinism (§3.3); pass `None` to leave it unset.
    ///
    /// This is the templating layer's output; subsequent phases parse the markup
    /// into the node tree without it ever crossing the public boundary.
    pub fn render_markup<T: Serialize>(
        &self,
        data: &T,
        now: Option<i64>,
    ) -> Result<(String, Diagnostics), RenderError> {
        functions::set_now(now);
        let result = self.render_inner(data);
        functions::set_now(None);
        result
            .map(|markup| (markup, Diagnostics::default()))
            .map_err(map_render_err)
    }

    fn render_inner<T: Serialize>(&self, data: &T) -> Result<String, minijinja::Error> {
        let template = self.env.get_template(MAIN)?;
        template.render(Value::from_serialize(data))
    }

    /// Render the template and parse the result into the resolved node tree
    /// (§1 Stage 1): Jinja render → html5ever parse → typed `t:` nodes. The
    /// intermediate markup never crosses the public boundary (the §1 hard rule).
    pub fn render_nodes<T: Serialize>(
        &self,
        data: &T,
        now: Option<i64>,
    ) -> Result<(Vec<Node>, Diagnostics), RenderError> {
        let (markup, diags) = self.render_markup(data, now)?;
        let nodes = markup::parse(&markup)?;
        Ok((nodes, diags))
    }

    /// Whether a `<t:running-header>` was extracted from the template source.
    pub fn has_header(&self) -> bool {
        self.has_header
    }

    /// Whether a `<t:running-footer>` was extracted from the template source.
    pub fn has_footer(&self) -> bool {
        self.has_footer
    }

    /// Render an extracted region (`HEADER`/`FOOTER`) against a per-page context
    /// and parse it into nodes (§6.6). `ctx` carries `page.*` and the original
    /// `data`; `now` pins the clock as for body rendering. Returns `None` when
    /// the named region was not present in the source.
    pub fn render_region<T: Serialize>(
        &self,
        name: &str,
        ctx: &T,
        now: Option<i64>,
    ) -> Option<Result<(Vec<Node>, Diagnostics), RenderError>> {
        let present = match name {
            HEADER => self.has_header,
            _ => self.has_footer,
        };
        if !present {
            return None;
        }
        Some(self.render_region_nodes(name, ctx, now))
    }

    fn render_region_nodes<T: Serialize>(
        &self,
        name: &str,
        ctx: &T,
        now: Option<i64>,
    ) -> Result<(Vec<Node>, Diagnostics), RenderError> {
        functions::set_now(now);
        let markup = self
            .env
            .get_template(name)
            .expect("region template registered at compile")
            .render(Value::from_serialize(ctx));
        functions::set_now(None);
        let markup = markup.map_err(map_render_err)?;
        let nodes = markup::parse(&markup)?;
        Ok((nodes, Diagnostics::default()))
    }
}

// Control-flow components. These map to **Jinja statements**, never to `t:`
// elements (a Jinja block tag is gone by the time html5ever sees the markup,
// whereas structure that must survive into the DOM uses `t:` elements — §2).
//
// Each component emits literal Jinja via sentinel tokens (see ./sentinel) so the
// expression strings survive React's HTML-escaping untouched. Attributes carry
// **expression strings**, not evaluated JS: `cond="total > 0"` stays the literal
// `total > 0`.

import { Fragment, type ReactNode } from "react";
import { sentinel } from "./sentinel.js";

/** A bare Jinja statement, e.g. `wrap("if total > 0")` -> `{% if total > 0 %}`. */
function statement(body: string): string {
  return sentinel(`{% ${body} %}`);
}

/** `{% if COND %} … {% endif %}` (§2.5). Chain with `<ElseIf>` / `<Else>`. */
export function If(props: { cond: string; children?: ReactNode }): ReactNode {
  return (
    <Fragment>
      {statement(`if ${props.cond}`)}
      {props.children}
      {statement("endif")}
    </Fragment>
  );
}

/** `{% elif COND %}` — a sibling inside the same `<If>` body (§2.5). */
export function ElseIf(props: { cond: string; children?: ReactNode }): ReactNode {
  return (
    <Fragment>
      {statement(`elif ${props.cond}`)}
      {props.children}
    </Fragment>
  );
}

/** `{% else %}` — a sibling inside the same `<If>` (or `<Each>`) body (§2.5/2.7). */
export function Else(props: { children?: ReactNode }): ReactNode {
  return (
    <Fragment>
      {statement("else")}
      {props.children}
    </Fragment>
  );
}

/**
 * `{% for AS in OF %} … {% endfor %}` (§2.7). When `index` is supplied, the body
 * is prefixed with `{% set <index> = loop.index0 %}` so authors get a named
 * 0-based index.
 *
 * TODO(phase11): stock Jinja has no per-name index binding in the `for` header —
 * the built-in `loop.index`/`loop.index0` are always available. We model the
 * optional `index` prop as a `{% set %}` of `loop.index0` at body top, which is
 * the closest portable equivalent.
 */
export function Each(props: {
  of: string;
  as: string;
  index?: string;
  children?: ReactNode;
}): ReactNode {
  return (
    <Fragment>
      {statement(`for ${props.as} in ${props.of}`)}
      {props.index ? statement(`set ${props.index} = loop.index0`) : null}
      {props.children}
      {statement("endfor")}
    </Fragment>
  );
}

/** `{% switch ON %} … {% endswitch %}` (§2.6). Children are `<Case>` / `<Default>`. */
export function Switch(props: { on: string; children?: ReactNode }): ReactNode {
  return (
    <Fragment>
      {statement(`switch ${props.on}`)}
      {props.children}
      {statement("endswitch")}
    </Fragment>
  );
}

/** `{% case V1, V2, … %}` — comma means membership; first match wins (§2.6). */
export function Case(props: { value: string; children?: ReactNode }): ReactNode {
  return (
    <Fragment>
      {statement(`case ${props.value}`)}
      {props.children}
    </Fragment>
  );
}

/** `{% default %}` — must be the last child of `<Switch>` (§2.6). */
export function Default(props: { children?: ReactNode }): ReactNode {
  return (
    <Fragment>
      {statement("default")}
      {props.children}
    </Fragment>
  );
}

/** Build the `include` statement, appending `with { … }` context when given. */
function includeStatement(src: string, withCtx?: string): string {
  return withCtx ? `include ${src} with ${withCtx}` : `include ${src}`;
}

/**
 * `{% include SRC %}` / `{% include SRC with CTX %}` (§2.8). `src` is an
 * expression string (usually a quoted partial name, e.g. `"remittance"`).
 */
export function Include(props: { src: string; with?: string }): ReactNode {
  return <Fragment>{statement(includeStatement(props.src, props.with))}</Fragment>;
}

/** `{{ EXPR }}` interpolation (§2.4). The expression is resolved in Rust at render. */
export function Expr(props: { value: string }): ReactNode {
  return <Fragment>{sentinel(`{{ ${props.value} }}`)}</Fragment>;
}

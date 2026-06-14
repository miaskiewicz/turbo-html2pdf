// `@turbo-pdf/template` — a deliberately tiny *second* authoring frontend.
//
// It exists to prove the core is frontend-agnostic (AC-8.9): a different tool, with
// no shared code, emits the *same* turbo-pdf template source string (Jinja control
// flow + `t:` directives + plain HTML) through the unchanged core. This one is a
// framework-free tagged-template / builder helper — no React, no JSX, ~one file.
//
// Unlike React, nothing here HTML-escapes, so there is no sentinel machinery: the
// helpers concatenate literal strings directly. The author is responsible for
// escaping plain text where needed (mirroring hand-authored HTML).

/** Anything that can be a child: a literal string or a nested builder result. */
export type Node = string | Node[];

/** Flatten a child (string or nested array) into a single source string. */
function render(child: Node): string {
  return Array.isArray(child) ? child.map(render).join("") : child;
}

/** Join a rest-args child list into one source string. */
function join(children: Node[]): string {
  return children.map(render).join("");
}

// ---------------------------------------------------------------------------
// Control flow -> Jinja statements
// ---------------------------------------------------------------------------

/** `{% if COND %} … {% endif %}` (§2.5). */
export function ifBlock(cond: string, ...children: Node[]): string {
  return `{% if ${cond} %}${join(children)}{% endif %}`;
}

/** `{% elif COND %} …` — place inside an `ifBlock` body (§2.5). */
export function elseIf(cond: string, ...children: Node[]): string {
  return `{% elif ${cond} %}${join(children)}`;
}

/** `{% else %} …` — place inside an `ifBlock`/`each` body (§2.5/2.7). */
export function elseBlock(...children: Node[]): string {
  return `{% else %}${join(children)}`;
}

/** `{% for AS in OF %} … {% endfor %}` (§2.7). */
export function each(of: string, as: string, ...children: Node[]): string {
  return `{% for ${as} in ${of} %}${join(children)}{% endfor %}`;
}

/** `{% switch ON %} … {% endswitch %}` (§2.6). */
export function switchBlock(on: string, ...children: Node[]): string {
  return `{% switch ${on} %}${join(children)}{% endswitch %}`;
}

/** `{% case V1, V2, … %} …` — inside a `switchBlock` (§2.6). */
export function caseBlock(value: string, ...children: Node[]): string {
  return `{% case ${value} %}${join(children)}`;
}

/** `{% default %} …` — last child of a `switchBlock` (§2.6). */
export function defaultBlock(...children: Node[]): string {
  return `{% default %}${join(children)}`;
}

/** `{% include SRC %}` / `{% include SRC with CTX %}` (§2.8). */
export function include(src: string, withCtx?: string): string {
  return withCtx ? `{% include ${src} with ${withCtx} %}` : `{% include ${src} %}`;
}

/** `{{ EXPR }}` interpolation (§2.4). */
export function expr(value: string): string {
  return `{{ ${value} }}`;
}

// ---------------------------------------------------------------------------
// Paged media -> `t:` directive elements (names per core's t_kind table)
// ---------------------------------------------------------------------------

/** Serialize an attribute map (skipping `undefined`) to ` name="value"` pairs. */
function attrs(map: Record<string, string | undefined>): string {
  let out = "";
  for (const [key, value] of Object.entries(map)) {
    if (value !== undefined) {
      out += ` ${key}="${value}"`;
    }
  }
  return out;
}

/**
 * Build a `t:`-prefixed directive element with attrs and optional children.
 *
 * Note the open/close pair even when empty (`<t:page></t:page>`): this matches the
 * exact bytes React's `renderToStaticMarkup` emits for custom elements, so the two
 * frontends are byte-identical (AC-8.9). html5ever parses this and the self-closing
 * `<t:page/>` form to the same node, so the choice is purely about string equality.
 */
function directive(
  name: string,
  attrMap: Record<string, string | undefined>,
  children?: Node[],
): string {
  const open = `t:${name}${attrs(attrMap)}`;
  return `<${open}>${children === undefined ? "" : join(children)}</t:${name}>`;
}

/** `<t:running-header>` (§3.0). */
export function runningHeader(opts: { extent?: string }, ...children: Node[]): string {
  return directive("running-header", { extent: opts.extent }, children);
}

/** `<t:running-footer>` (§3.0). */
export function runningFooter(opts: { extent?: string }, ...children: Node[]): string {
  return directive("running-footer", { extent: opts.extent }, children);
}

/** `<t:page-master>` (§3.1). */
export function pageMaster(
  opts: { name: string; size?: string; orientation?: string; margin?: string },
  ...children: Node[]
): string {
  return directive("page-master", opts, children);
}

/** `<t:region slot extent>` (§3.1). */
export function region(opts: { slot: string; extent?: string }, ...children: Node[]): string {
  return directive("region", opts, children);
}

/** `<t:variant kind>` (§3.2). */
export function variant(kind: string, ...children: Node[]): string {
  return directive("variant", { kind }, children);
}

/** `<t:use-master name/>` (§3.4). */
export function useMaster(name: string): string {
  return directive("use-master", { name });
}

/** `<t:footnote>` (§3.6). */
export function footnote(opts: { mark?: string; reset?: string }, ...children: Node[]): string {
  return directive("footnote", { mark: opts.mark, "t:footnote-reset": opts.reset }, children);
}

/** `<t:counter name action step start/>` (§3.8). */
export function counter(opts: {
  name: string;
  action?: string;
  step?: string;
  start?: string;
}): string {
  return directive("counter", opts);
}

/** `<t:leader/>` (§3.10). */
export function leader(): string {
  return directive("leader", {});
}

/** A named running element via the `t:running` attribute on a host element (§3.5). */
export function running(
  opts: { name: string; tag?: string; policy?: string },
  ...children: Node[]
): string {
  const tag = opts.tag ?? "span";
  const map = attrs({ "t:running": opts.name, "t:running-policy": opts.policy });
  return `<${tag}${map}>${join(children)}</${tag}>`;
}

/** `<t:page/>` (§3.3). */
export function page(): string {
  return directive("page", {});
}

/** `<t:pages/>` (§3.3). */
export function pages(): string {
  return directive("pages", {});
}

/** Final assembly: concatenate top-level nodes and trim outer whitespace. */
export function compileTemplate(...children: Node[]): string {
  return join(children).trim();
}

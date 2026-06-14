// The public authoring entry point: turn a React element tree into a turbo-pdf
// template *source string* (Jinja control flow + `t:` paged-media directives +
// plain HTML). React runs **once at authoring time** here via `renderToStaticMarkup`;
// it is never on the render hot path and has no access to render data (§8.4).

import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expandSentinels } from "./sentinel.js";

/** Options for {@link compileTemplate}. */
export interface CompileOptions {
  /**
   * Trim insignificant leading/trailing whitespace from the final string. On by
   * default; this is the only normalization applied (the interior is verbatim).
   */
  trim?: boolean;
}

/**
 * Render `element` to a turbo-pdf template source string.
 *
 * Pipeline: `renderToStaticMarkup` produces HTML where plain elements and `t:`
 * directives are emitted (and legitimately HTML-escaped) as-is, while control-flow
 * components emitted sentinel tokens carrying their literal Jinja. `expandSentinels`
 * then decodes those tokens into real `{% … %}` / `{{ … }}` text — the one
 * post-process pass that undoes nothing else, so the result is byte-stable.
 */
export function compileTemplate(element: ReactElement, options: CompileOptions = {}): string {
  const markup = renderToStaticMarkup(element);
  const expanded = expandSentinels(markup);
  return options.trim === false ? expanded : expanded.trim();
}

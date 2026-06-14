// Raw passthrough escape hatch.
//
// React's JSX intrinsics impose their own attribute model: `class`/`colspan` warn
// (they want `className`/`colSpan`, though they still render), and `style` MUST be
// an object — a `style="color:#111"` *string* is a hard error. turbo-pdf templates,
// by contrast, are plain HTML where `class`, `style="…"`, and `t:style` are ordinary
// string attributes (spec §8.4). `<Raw html="…" />` lets an author drop a literal
// markup fragment that bypasses React's attribute model entirely: the string is
// emitted verbatim (un-escaped) via the same sentinel channel the control-flow
// components use, so whatever you write is exactly what lands in the template.
//
// The author owns well-formedness here (just like hand-authored HTML); the markup is
// re-parsed by html5ever in the core.

import { createElement, Fragment, type ReactElement } from "react";
import { sentinel } from "./sentinel.js";

/** Emit `props.html` verbatim into the template (no HTML-escaping, no React model). */
export function Raw(props: { html: string }): ReactElement {
  return createElement(Fragment, null, sentinel(props.html));
}

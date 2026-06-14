// Paged-media components. These map to the `t:` directive elements that must
// survive into the parsed DOM as typed layout nodes (§3). The element names match
// `turbo-pdf-core`'s authoritative `t_kind()` table exactly (node.rs).
//
// We build these with `createElement(string, …)` rather than JSX, because a JSX
// tag like `<t:running-header>` parses as the member expression `t.running` — the
// `t:` namespace prefix is only valid as a *string* tag name. React DOM renders an
// arbitrary string tag verbatim, and `renderToStaticMarkup` HTML-escapes the
// attribute values and text children exactly as a hand-written template would, so
// the `t:` output is byte-identical to hand-authored markup.

import { createElement, type ReactElement, type ReactNode } from "react";

/** Drop `undefined`-valued attributes so they don't render as `attr="undefined"`. */
function defined(attrs: Record<string, string | undefined>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(attrs)) {
    if (value !== undefined) {
      out[key] = value;
    }
  }
  return out;
}

/** Create a `t:`-prefixed directive element with the given attrs and children. */
function directive(
  name: string,
  attrs: Record<string, string | undefined>,
  children?: ReactNode,
): ReactElement {
  return createElement(`t:${name}`, defined(attrs), children);
}

/** `<t:running-header>` — attaches header content to every page's top band (§3.0). */
export function RunningHeader(props: { extent?: string; children?: ReactNode }): ReactElement {
  return directive("running-header", { extent: props.extent }, props.children);
}

/** `<t:running-footer>` — attaches footer content to every page's bottom band (§3.0). */
export function RunningFooter(props: { extent?: string; children?: ReactNode }): ReactElement {
  return directive("running-footer", { extent: props.extent }, props.children);
}

/** `<t:page-master>` — named geometry + region bundle (§3.1). */
export function PageMaster(props: {
  name: string;
  size?: string;
  orientation?: string;
  margin?: string;
  children?: ReactNode;
}): ReactElement {
  return directive(
    "page-master",
    {
      name: props.name,
      size: props.size,
      orientation: props.orientation,
      margin: props.margin,
    },
    props.children,
  );
}

/** `<t:region slot extent>` — a margin-box region inside a master (§3.1). */
export function Region(props: {
  slot: string;
  extent?: string;
  children?: ReactNode;
}): ReactElement {
  return directive("region", { slot: props.slot, extent: props.extent }, props.children);
}

/** `<t:variant kind>` — first/left/right/blank override inside a master (§3.2). */
export function Variant(props: { kind: string; children?: ReactNode }): ReactElement {
  return directive("variant", { kind: props.kind }, props.children);
}

/** `<t:use-master name>` — switches the active master from flow position (§3.4). */
export function UseMaster(props: { name: string }): ReactElement {
  return directive("use-master", { name: props.name });
}

/** `<t:footnote>` — inline footnote reference + body (§3.6). */
export function Footnote(props: {
  mark?: string;
  reset?: string;
  children?: ReactNode;
}): ReactElement {
  return directive(
    "footnote",
    { mark: props.mark, "t:footnote-reset": props.reset },
    props.children,
  );
}

/** `<t:footnote-separator>` — the rule between body and footnote area (§3.6). */
export function FootnoteSeparator(props: { children?: ReactNode }): ReactElement {
  return directive("footnote-separator", {}, props.children);
}

/** `<t:counter name action step start>` — a general named counter (§3.8). */
export function Counter(props: {
  name: string;
  action?: string;
  step?: string;
  start?: string;
}): ReactElement {
  return directive("counter", {
    name: props.name,
    action: props.action,
    step: props.step,
    start: props.start,
  });
}

/** `<t:leader>` — dot-fill between two inline boxes, for TOCs/footers (§3.10). */
export function Leader(props: { children?: ReactNode }): ReactElement {
  return directive("leader", {}, props.children);
}

/** `<t:anchor id>` — a cross-reference target (§3.9, feature-gated in core). */
export function Anchor(props: { id: string }): ReactElement {
  return directive("anchor", { id: props.id });
}

/** `<t:endnote>` — collects to a `<t:endnotes/>` sink (§3.7, feature-gated). */
export function Endnote(props: { children?: ReactNode }): ReactElement {
  return directive("endnote", {}, props.children);
}

/** `<t:endnotes/>` — the endnote sink (§3.7, feature-gated). */
export function Endnotes(): ReactElement {
  return directive("endnotes", {});
}

/**
 * Named running element (§3.5). There is **no** `t:running` element in the core's
 * authoritative `t_kind()` table — running content is marked with the
 * `t:running="name"` *attribute* on a normal element (`<h1 t:running="section">`).
 * So `<Running name="section">` renders that attribute on a host element (default
 * `<span>`, override via `as`), optionally with `t:running-policy`.
 *
 * TODO(phase11): the handoff sketched `<Running>` -> `<t:running>`, but node.rs
 * has no such directive; spec §3.5 defines it as an attribute, which is what we emit.
 */
export function Running(props: {
  name: string;
  as?: string;
  policy?: string;
  children?: ReactNode;
}): ReactElement {
  return createElement(
    props.as ?? "span",
    defined({ "t:running": props.name, "t:running-policy": props.policy }),
    props.children,
  );
}

/** `<t:page/>` — current 1-based page number convenience element (§3.3). */
export function Page(): ReactElement {
  return directive("page", {});
}

/** `<t:pages/>` — total page count convenience element (§3.3). */
export function Pages(): ReactElement {
  return directive("pages", {});
}

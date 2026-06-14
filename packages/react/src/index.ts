// `@turbo-pdf/react` — the first authoring frontend for turbo-pdf.
//
// React components that render *once at authoring time* to a turbo-pdf template
// source string (Jinja control flow + `t:` paged-media directives + plain HTML).
// React is never on the render hot path; expressions are strings resolved in Rust
// at render. See ./compile for the entry point and ./sentinel for how literal Jinja
// survives React's HTML-escaping.

export { compileTemplate, type CompileOptions } from "./compile.js";
export {
  Case,
  Default,
  Each,
  Else,
  ElseIf,
  Expr,
  If,
  Include,
  Switch,
} from "./control-flow.js";
export { Raw } from "./raw.js";
export {
  Anchor,
  Counter,
  Endnote,
  Endnotes,
  Footnote,
  FootnoteSeparator,
  Leader,
  Page,
  PageMaster,
  Pages,
  Region,
  Running,
  RunningFooter,
  RunningHeader,
  UseMaster,
  Variant,
} from "./paged-media.js";

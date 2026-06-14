/* turbo-pdf N-API binding — TypeScript surface (Phase 10).
 *
 * Hand-maintained to mirror the `#[napi]` exports in `src/lib.rs`. When the repo
 * gains a `napi build` step in CI this can be regenerated; the shapes below are
 * the contract callers depend on. */

/** A source location: 1-based line/column (0 when unknown) and byte offset. */
export interface Span {
  line: number
  col: number
  byteOffset: number
}

/** A fatal compile/render fault. Thrown by `compile`, `program.render`, and the
 *  one-shot `render`. Non-fatal lints are NOT thrown — they come back in
 *  `RenderResult.diagnostics`. */
export class TurboPdfError extends Error {
  name: 'TurboPdfError'
  /** Stable machine-readable code, e.g. `"TemplateSyntax"`, `"UndefinedValue"`. */
  code: string
  /** Source span of the offending construct. */
  span: Span
}

/** A non-fatal diagnostic (lint) collected during render. */
export interface Diagnostic {
  /** Stable lint code, e.g. `"UnsupportedCss"`, `"RegionOverflow"`. */
  code: string
  message: string
  /** 1-based source line (0 when unknown). */
  line: number
  /** 1-based source column (0 when unknown). */
  col: number
}

/** PDF document-info metadata. Every field is optional. */
export interface DocMeta {
  title?: string
  author?: string
  subject?: string
  keywords?: string
  /** Creation date as Unix seconds. Omit for the reproducible sentinel date. */
  creationDate?: number
}

/** Options for a single render pass. All fields optional. */
export interface RenderOptions {
  /** Data object interpolated into the template (`{{ data.* }}`). */
  data?: unknown
  /** Author CSS; also supplies `@page` geometry (size/margins). */
  css?: string
  /** Font programs (raw OpenType/TrueType bytes), one Buffer per face. */
  fonts?: Buffer[]
  /** Raster images. Accepted but not yet embedded (Phase 9b). */
  images?: Buffer[]
  /** PDF document metadata. */
  meta?: DocMeta
  /** Pins the `now()` clock (Unix seconds) for deterministic output. */
  now?: number
}

/** The result of a render. */
export interface RenderResult {
  /** The rendered PDF 1.7 document. */
  pdf: Buffer
  /** Non-fatal diagnostics (lints) — never thrown. */
  diagnostics: Diagnostic[]
  /** Number of pages. */
  pageCount: number
}

/** A compiled, reusable template program (thread-safe native handle). */
export interface Program {
  /** Render this program against `opts`. Throws `TurboPdfError` on a fatal fault. */
  render(opts?: RenderOptions): RenderResult
  /** Whether the source declared a `<t:running-header>`. */
  hasHeader(): boolean
  /** Whether the source declared a `<t:running-footer>`. */
  hasFooter(): boolean
}

/** Compile a template into a reusable {@link Program}. Throws `TurboPdfError`. */
export function compile(templateHtml: string, opts?: unknown): Program

/** One-shot convenience: compile + render in a single call. */
export function render(templateHtml: string, opts?: RenderOptions): RenderResult

/* turbo-pdf N-API binding — TypeScript surface (Phase 10).
 *
 * Hand-maintained to mirror the `#[napi]` exports in `src/lib.rs`. The shapes
 * below are the contract callers depend on — richer than napi's auto-codegen
 * (`Span`, `TurboPdfError`, `CompileOptions`, `unknown` over `any`).
 *
 * Do NOT let `napi build` overwrite this file. The `build`/`build:debug` scripts
 * redirect codegen to `index.generated.d.ts` (git-ignored) via `--dts`. If you
 * run `napi build` by hand, pass `--dts index.generated.d.ts` too. */

/** A source location: 1-based line/column (0 when unknown) and byte offset. */
export interface Span {
  line: number;
  col: number;
  byteOffset: number;
}

/** A fatal compile/render fault. Thrown by `compile`, `program.render`, and the
 *  one-shot `render`. Non-fatal lints are NOT thrown — they come back in
 *  `RenderResult.diagnostics`. */
export class TurboPdfError extends Error {
  name: "TurboPdfError";
  /** Stable machine-readable code, e.g. `"TemplateSyntax"`, `"UndefinedValue"`. */
  code: string;
  /** Source span of the offending construct. */
  span: Span;
}

/** A non-fatal diagnostic (lint) collected during render. */
export interface Diagnostic {
  /** Stable lint code, e.g. `"UnsupportedCss"`, `"RegionOverflow"`. */
  code: string;
  message: string;
  /** 1-based source line (0 when unknown). */
  line: number;
  /** 1-based source column (0 when unknown). */
  col: number;
}

/** PDF document-info metadata. Every field is optional. */
export interface DocMeta {
  title?: string;
  author?: string;
  subject?: string;
  keywords?: string;
  /** Creation date as Unix seconds. Omit for the reproducible sentinel date. */
  creationDate?: number;
}

/** One font face: the program bytes plus the selection metadata the cascade
 *  matches `font-family` / `font-weight` / italic against. */
export interface FontFace {
  /** The font program bytes (`.ttf`/`.otf`). */
  data: Buffer;
  /** The CSS `font-family` name this face answers to. */
  family: string;
  /** CSS `font-weight` (100..=900). Defaults to 400 (normal). */
  weight?: number;
  /** Whether this is the italic/oblique face. Defaults to `false`. */
  italic?: boolean;
}

/** One named raster image: a template name plus its encoded PNG/JPEG bytes.
 *  `<img src="name">` / `background-image: url(name)` embeds the matching
 *  bytes as a PDF image XObject. */
export interface NamedImage {
  /** The name the template refers to this image by. */
  name: string;
  /** The encoded image bytes (PNG or JPEG). */
  data: Buffer;
}

/** A page watermark stamped behind the body on every page. Either a shaped-word
 *  text mark or a raster image mark (resolved by name through
 *  `RenderOptions.images`); set `image` for the raster form. */
export interface Watermark {
  /** Word to stamp (text mark). Defaults to `DRAFT`. */
  text?: string;
  /** Fill color `#rrggbb` (text mark). Defaults to gray. */
  color?: string;
  /** Image name (image mark), resolved against `RenderOptions.images`. */
  image?: string;
  /** Fill opacity `0.0..=1.0`. Defaults to 0.25 (text) / 1.0 (image). */
  opacity?: number;
  /** Rotation in degrees (text mark). Defaults to 45. */
  angle?: number;
  /** Tile the image mark across the page instead of centering it. */
  tiled?: boolean;
}

/** Compile-time knobs: partials, missing-value policy, and include depth. */
export interface CompileOptions {
  /** Partial templates by name, for `{% include %}`. */
  partials?: Record<string, string>;
  /** `"strict"` (default) throws on a missing value; `"empty"`/`"lenient"`
   *  renders it as empty. */
  missingPolicy?: "strict" | "empty" | "lenient";
  /** Maximum `{% include %}` nesting depth (defaults to the core default). */
  includeMaxDepth?: number;
}

/** AES-256 password-encryption settings. `userPassword` is required to open the
 *  document; `ownerPassword` (when set) grants full permissions. Permission bits
 *  default to all-granted; set one to `false` to clear it for a user-password
 *  open. Encrypted output is intentionally non-deterministic. */
export interface Encryption {
  /** Password required to open the document. */
  userPassword: string;
  /** Owner password granting full permissions. Defaults to the user password. */
  ownerPassword?: string;
  /** Allow printing. Defaults to `true`. */
  print?: boolean;
  /** Allow modifying contents. Defaults to `true`. */
  modify?: boolean;
  /** Allow copying/extracting text and graphics. Defaults to `true`. */
  copy?: boolean;
  /** Allow adding/modifying annotations. Defaults to `true`. */
  annotate?: boolean;
  /** Allow filling existing form fields. Defaults to `true`. */
  fillForms?: boolean;
  /** Allow accessibility text extraction. Defaults to `true`. */
  accessibility?: boolean;
  /** Allow document assembly (insert/rotate/delete pages). Defaults to `true`. */
  assemble?: boolean;
  /** Allow full-resolution printing. Defaults to `true`. */
  highQualityPrint?: boolean;
}

/** Options for a single render pass. All fields optional. */
export interface RenderOptions {
  /** Data object interpolated into the template (`{{ data.* }}`). */
  data?: unknown;
  /** Author CSS; also supplies `@page` geometry (size/margins). */
  css?: string;
  /** Font faces, each `{ data, family, weight?, italic? }`. */
  fonts?: FontFace[];
  /** Named raster images embedded by template name. */
  images?: NamedImage[];
  /** PDF document metadata. */
  meta?: DocMeta;
  /** A faded watermark stamped behind the body on every page. */
  watermark?: Watermark;
  /** Pins the `now()` clock (Unix seconds) for deterministic output. */
  now?: number;
  /** Emit PDF/A-2b archival conformance (sRGB `/OutputIntents`, XMP `pdfaid`,
   *  trailer `/ID`). Defaults to `false`. */
  pdfA?: boolean;
  /** Emit tagged / accessible PDF/UA-1 machinery (`/StructTreeRoot`, `/MarkInfo`,
   *  per-face `/ToUnicode`, `/Lang`). Defaults to `false`. */
  pdfUa?: boolean;
  /** Natural-language tag (RFC 3066, e.g. `en-US`) written as the catalog
   *  `/Lang`. Only meaningful with `pdfUa`. */
  lang?: string;
  /** Emit fills in DeviceCMYK instead of DeviceRGB. Defaults to `false`. */
  cmyk?: boolean;
  /** AES-256 password protection. Omit for an unencrypted, byte-deterministic
   *  PDF. */
  encryption?: Encryption;
  /** Foreign PDF documents glued (page by page) AFTER the rendered pages, in
   *  order. Each entry is a complete PDF. */
  appendPdfs?: Buffer[];
}

/** The result of a render. */
export interface RenderResult {
  /** The rendered PDF 1.7 document. */
  pdf: Buffer;
  /** Non-fatal diagnostics (lints) — never thrown. */
  diagnostics: Diagnostic[];
  /** Number of pages. */
  pageCount: number;
}

/** A reusable, pre-parsed set of fonts. Build it ONCE — e.g. warm it at server
 *  startup — and pass the handle to every `render` call so the font programs are
 *  parsed once instead of on every request. Omit to fall back to per-call
 *  `RenderOptions.fonts`. */
export class Fonts {
  /** Parse `fonts` (each `{ data, family, weight?, italic? }`) once into a
   *  reusable handle. Do this at startup, then reuse. */
  static load(fonts: FontFace[]): Fonts;
}

/** A compiled, reusable template program (thread-safe native handle). */
export interface Program {
  /** Render this program. Pass a prebuilt {@link Fonts} handle to reuse parsed
   *  fonts across calls. Throws `TurboPdfError` on a fatal fault. */
  render(opts?: RenderOptions, fonts?: Fonts): RenderResult;
  /** Whether the source declared a `<t:running-header>`. */
  hasHeader(): boolean;
  /** Whether the source declared a `<t:running-footer>`. */
  hasFooter(): boolean;
}

/** Compile a template into a reusable {@link Program}. Throws `TurboPdfError`. */
export function compile(templateHtml: string, opts?: CompileOptions): Program;

/** One-shot convenience: compile + render in a single call. Pass a prebuilt
 *  {@link Fonts} handle to reuse parsed fonts. */
export function render(templateHtml: string, opts?: RenderOptions, fonts?: Fonts): RenderResult;

/** Glue one or more foreign PDF documents after `base`, page by page, returning
 *  the merged PDF. Equivalent to `RenderOptions.appendPdfs` but usable on
 *  already-emitted bytes. Throws `TurboPdfError` if any input fails to parse. */
export function appendPdf(base: Buffer, extras: Buffer[]): Buffer;

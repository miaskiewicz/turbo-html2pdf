// Shared contracts for the competitive benchmark harness (Phase 14, spec §10.2).
//
// Every engine adapter implements the same `EngineAdapter` interface so the runner
// can drive them uniformly. Workloads describe EQUIVALENT logical documents — same
// fonts, same page geometry, same text — expressed per-engine.

/** Page geometry shared by every engine so outputs are comparable (AC-10.10). */
export interface PageGeometry {
  /** Page size name. A4 keeps every engine on identical paper. */
  readonly size: "A4";
  readonly widthPt: number;
  readonly heightPt: number;
  /** Uniform margin in points. */
  readonly marginPt: number;
}

/** A single table row in the generated content. */
export interface RowData {
  readonly id: number;
  readonly description: string;
  readonly qty: number;
  readonly unit: number;
  readonly amount: number;
}

/** Deterministic content for a workload, generated from a fixed seed. */
export interface WorkloadContent {
  readonly title: string;
  readonly subtitle: string;
  readonly currency: string;
  readonly rows: readonly RowData[];
  /** Flowing paragraphs (legal/mixed workloads). */
  readonly paragraphs: readonly string[];
  /** Footnote bodies keyed by marker index (legal workload). */
  readonly footnotes: readonly string[];
  readonly total: number;
}

/** Identifies one logical document used across all engines. */
export type WorkloadId = "invoice" | "report-100" | "report-1k" | "report-10k" | "legal" | "mixed";

/** A workload = id + geometry + deterministically generated content. */
export interface Workload {
  readonly id: WorkloadId;
  readonly geometry: PageGeometry;
  readonly content: WorkloadContent;
}

/** Fine-grained timing for a single render call (milliseconds). */
export interface RenderTimings {
  /** Wall-clock for the render call itself. */
  readonly renderMs: number;
  /** Optional engine/process init cost folded into this call (cold runs). */
  readonly initMs?: number;
}

/** Result of rendering one workload with one engine. */
export interface RenderResult {
  readonly pdfBytes: Uint8Array;
  readonly timings: RenderTimings;
}

/** Why an engine is unavailable, for honest reporting. */
export interface Availability {
  readonly available: boolean;
  readonly version: string | null;
  /** Human-readable reason when unavailable. */
  readonly reason: string | null;
}

/** Distribution footprint per engine (AC-10.11 reporting). */
export interface Footprint {
  /** Installed dependency size in bytes, or null if unknown. */
  readonly installedBytes: number | null;
  /** Whether the engine ships or downloads a full browser. */
  readonly shipsBrowser: boolean;
  readonly notes: string;
}

/**
 * Uniform engine interface. Implementations MUST detect availability and never
 * hard-fail the harness when their dependency is missing.
 */
export interface EngineAdapter {
  readonly id: string;
  /** Approach group: browser / react / draw-api / typesetting / reference / wip. */
  readonly kind: "browser" | "react" | "draw-api" | "typesetting" | "reference" | "wip";
  /** Detect whether this engine can run on this machine. */
  detect(): Promise<Availability>;
  /** Static distribution footprint metadata. */
  footprint(): Footprint;
  /** Cold render: spin engine up, render once, tear down. */
  renderCold(workload: Workload): Promise<RenderResult>;
  /** Warm render: reuse a pooled engine/process where applicable. */
  renderWarm(workload: Workload): Promise<RenderResult>;
  /** Release any pooled resources (browser, process pool). */
  dispose(): Promise<void>;
}

// Core measurement loop for one (engine × workload). Runs warmup, then N measured
// cold and warm renders, sampling peak RSS, plus a small throughput probe.

import { availableParallelism } from "node:os";
import type { EngineAdapter, RenderResult, Workload } from "../types.ts";
import { type Summary, summarize } from "./stats.ts";
import { assertWorkloadPdf } from "../compare/content.ts";

export interface MeasureOpts {
  readonly runs: number;
  readonly warmup: number;
}

export interface MeasureResult {
  readonly cold: Summary;
  readonly warm: Summary;
  readonly initMs: Summary;
  readonly pdfBytes: number;
  readonly peakRssBytes: number;
  readonly throughputPerSec: number;
}

type RenderFn = (w: Workload) => Promise<RenderResult>;

async function collect(fn: RenderFn, w: Workload, runs: number, peak: { v: number }) {
  const render: number[] = [];
  const init: number[] = [];
  let bytes = 0;
  for (let i = 0; i < runs; i++) {
    const r = await fn(w);
    render.push(r.timings.renderMs);
    if (r.timings.initMs !== undefined) init.push(r.timings.initMs);
    bytes = r.pdfBytes.length;
    const rss = process.memoryUsage().rss;
    if (rss > peak.v) peak.v = rss;
  }
  return { render, init, bytes };
}

async function warmUp(fn: RenderFn, w: Workload, n: number): Promise<void> {
  for (let i = 0; i < n; i++) await fn(w);
}

/** Heavy workloads cap the concurrent batch so probes stay tractable. */
function batchCount(rows: number): number {
  const cores = availableParallelism();
  // TODO(phase14): tune once turbo-pdf is in the matrix; heavy docs dominate runtime.
  return rows >= 1000 ? cores : cores * 2;
}

/** Throughput: render `count` docs concurrently up to core count, docs/sec. */
async function throughput(fn: RenderFn, w: Workload): Promise<number> {
  const count = batchCount(w.content.rows.length);
  const start = process.hrtime.bigint();
  const batch = Array.from({ length: count }, () => fn(w));
  await Promise.all(batch);
  const sec = Number(process.hrtime.bigint() - start) / 1e9;
  return sec > 0 ? count / sec : 0;
}

/** Measure one engine on one workload. Throws if the engine cannot render it. */
export async function measure(
  engine: EngineAdapter,
  w: Workload,
  opts: MeasureOpts,
): Promise<MeasureResult> {
  // Admit both paths outside every timer: they may render different documents.
  await assertWorkloadPdf((await engine.renderWarm(w)).pdfBytes, w);
  await assertWorkloadPdf((await engine.renderCold(w)).pdfBytes, w);
  const peak = { v: 0 };
  await warmUp((x) => engine.renderWarm(x), w, opts.warmup);
  const cold = await collect((x) => engine.renderCold(x), w, opts.runs, peak);
  const warm = await collect((x) => engine.renderWarm(x), w, opts.runs, peak);
  const tput = await throughput((x) => engine.renderWarm(x), w);
  return {
    cold: summarize(cold.render),
    warm: summarize(warm.render),
    initMs: summarize(cold.init),
    pdfBytes: warm.bytes,
    peakRssBytes: peak.v,
    throughputPerSec: tput,
  };
}

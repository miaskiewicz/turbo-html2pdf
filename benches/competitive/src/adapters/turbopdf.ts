// turbo-html2pdf adapter — drives the native N-API binding (turbo-html2pdf).
//
// Cold render = compile the (static) HTML body + render, fonts parsed per call
// (a fresh registry) — the true one-shot cost, no browser to spin up. Warm render
// = reuse the compiled `Program` AND a prebuilt `Fonts` handle (parse fonts once),
// which is turbo-html2pdf's amortization story (spec AC-10.4).

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Availability, EngineAdapter, Footprint, RenderResult, Workload } from "../types.ts";
import { FONTS } from "../workloads/fonts.ts";
import { bodyHtml, layoutCss } from "../workloads/html.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
// benches/competitive/src/adapters -> repo root -> the napi package entry.
const NAPI_ENTRY = resolve(HERE, "../../../../crates/turbo-pdf-napi/index.js");
const require = createRequire(import.meta.url);

interface Program {
  render(opts: Record<string, unknown>, fonts?: unknown): { pdf: Buffer; pageCount: number };
}
interface Napi {
  compile(html: string, opts?: unknown): Program;
  Fonts: { load(fonts: Buffer[]): unknown };
}

export class TurboPdfAdapter implements EngineAdapter {
  readonly id = "turbo-html2pdf";
  readonly kind = "typesetting" as const;

  private napi: Napi | null = null;
  private fontBytes: Buffer[] | null = null;
  private fontsHandle: unknown = null;
  private readonly programs = new Map<string, Program>();

  private load(): Napi {
    if (!this.napi) this.napi = require(NAPI_ENTRY) as Napi;
    return this.napi;
  }

  private fonts(): Buffer[] {
    if (!this.fontBytes) this.fontBytes = [readFileSync(FONTS.regular), readFileSync(FONTS.bold)];
    return this.fontBytes;
  }

  detect(): Promise<Availability> {
    try {
      const napi = this.load();
      const out = napi.compile("<p>ok</p>").render({ css: "", fonts: this.fonts(), now: 0 });
      const ok = out.pdf.length > 0 && out.pdf.subarray(0, 5).toString() === "%PDF-";
      return Promise.resolve(
        ok
          ? { available: true, version: "0.2.1", reason: null }
          : { available: false, version: null, reason: "napi produced no PDF" },
      );
    } catch (e) {
      return Promise.resolve({
        available: false,
        version: null,
        reason: `napi not built (run: cd crates/turbo-pdf-napi && pnpm build:cargo): ${(e as Error).message}`,
      });
    }
  }

  footprint(): Footprint {
    return {
      installedBytes: null,
      shipsBrowser: false,
      notes: "native N-API addon (~few MB), no browser/Chromium download",
    };
  }

  renderCold(w: Workload): Promise<RenderResult> {
    const napi = this.load();
    const t = performance.now();
    const program = napi.compile(bodyHtml(w));
    const { pdf } = program.render({ css: layoutCss(w), fonts: this.fonts(), now: 0 });
    const renderMs = performance.now() - t;
    return Promise.resolve({ pdfBytes: new Uint8Array(pdf), timings: { renderMs } });
  }

  renderWarm(w: Workload): Promise<RenderResult> {
    const napi = this.load();
    if (!this.fontsHandle) this.fontsHandle = napi.Fonts.load(this.fonts());
    let program = this.programs.get(w.id);
    if (!program) {
      program = napi.compile(bodyHtml(w));
      this.programs.set(w.id, program);
    }
    const t = performance.now();
    const { pdf } = program.render({ css: layoutCss(w), now: 0 }, this.fontsHandle);
    const renderMs = performance.now() - t;
    return Promise.resolve({ pdfBytes: new Uint8Array(pdf), timings: { renderMs } });
  }

  dispose(): Promise<void> {
    this.programs.clear();
    this.fontsHandle = null;
    return Promise.resolve();
  }
}

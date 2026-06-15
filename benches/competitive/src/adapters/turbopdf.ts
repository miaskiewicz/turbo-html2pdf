// turbo-pdf adapter — STUB.
//
// TODO(phase14): wire the real adapter once Phase 10 (N-API binding) lands. The
// binding will expose `compile(template) -> Program` and `program.render(data) ->
// Uint8Array`. This adapter should: compile the workload's `t:`-DSL template once
// (cold = include compile, warm = reuse the cached Program — that is turbo-pdf's
// headline amortization story, spec AC-10.4), then render per run. Until then the
// adapter advertises itself as unavailable so the harness still runs end-to-end.

import type { Availability, EngineAdapter, Footprint, RenderResult, Workload } from "../types.ts";

const PENDING = "pending Phase 10 napi (@turbo-html2pdf/core not built yet)";

export class TurboPdfAdapter implements EngineAdapter {
  readonly id = "turbo-pdf";
  readonly kind = "wip" as const;

  detect(): Promise<Availability> {
    // TODO(phase14): replace with a real `import('@turbo-html2pdf/core')` probe.
    return Promise.resolve({ available: false, version: null, reason: PENDING });
  }

  footprint(): Footprint {
    return {
      installedBytes: null,
      shipsBrowser: false,
      notes: "native N-API addon (~few MB), no browser — measured after Phase 10",
    };
  }

  renderCold(_w: Workload): Promise<RenderResult> {
    return Promise.reject(new Error(PENDING));
  }

  renderWarm(_w: Workload): Promise<RenderResult> {
    return Promise.reject(new Error(PENDING));
  }

  async dispose(): Promise<void> {
    // No state until Phase 10.
  }
}

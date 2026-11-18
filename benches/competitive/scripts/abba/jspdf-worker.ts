import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { JspdfAdapter } from "../../src/adapters/jspdf.ts";
import { getWorkload } from "../../src/workloads/index.ts";
import { FONTS } from "../../src/workloads/fonts.ts";
import { assertWorkloadText } from "../../src/compare/content.ts";
import { summarize } from "../../src/runner/stats.ts";

const [outArg, runsArg = "100", idsArg = "invoice,report-100,legal,mixed"] = process.argv.slice(2);
if (!outArg) throw new Error("usage: jspdf-worker.ts OUT RUNS IDS");
const out = resolve(outArg);
const runs = Number(runsArg);
if (!Number.isInteger(runs) || runs < 0) throw new Error("invalid runs");
mkdirSync(out, { recursive: true });
const engine = new JspdfAdapter();
const availability = await engine.detect();
if (!availability.available) throw new Error(availability.reason ?? "jsPDF unavailable");
const results = [];
for (const id of idsArg.split(",")) {
  const workload = getWorkload(id as never);
  const preflights = [];
  const modes: Record<string, unknown> = {};
  for (const mode of ["warm", "cold"] as const) {
    const render = () =>
      mode === "cold" ? engine.renderCold(workload) : engine.renderWarm(workload);
    const rendered = await render();
    const path = resolve(out, `${id}-${mode}.pdf`);
    writeFileSync(path, rendered.pdfBytes);
    const text = execFileSync("pdftotext", ["-enc", "UTF-8", path, "-"], {
      encoding: "utf8",
      timeout: 30000,
    });
    assertWorkloadText(text, workload);
    const info = execFileSync("pdfinfo", [path], { encoding: "utf8", timeout: 30000 });
    const pages = Number(info.match(/^Pages:\s+(\d+)/m)?.[1]);
    if (!Number.isInteger(pages) || pages < 1) throw new Error(`invalid PDF page count ${id}`);
    preflights.push({ mode, pages, bytes: rendered.pdfBytes.length, contentValidated: true });
    if (runs > 0) {
      for (let i = 0; i < 10; i++) await render();
      const samples = [];
      for (let i = 0; i < runs; i++) {
        const result = await render();
        if (!result.pdfBytes.length) throw new Error("empty timed PDF");
        samples.push(result.timings.renderMs);
      }
      modes[mode] = { summary: summarize(samples), samples };
    }
  }
  results.push({ id, preflights, modes });
}
const require = createRequire(import.meta.url);
function packageVersion(name: string): string {
  let dir = dirname(require.resolve(name));
  while (dirname(dir) !== dir) {
    const path = resolve(dir, "package.json");
    if (existsSync(path)) {
      const metadata = JSON.parse(readFileSync(path, "utf8"));
      if (metadata.name === name) return metadata.version;
    }
    dir = dirname(dir);
  }
  throw new Error(`missing package metadata ${name}`);
}
const packages = Object.fromEntries(
  ["jspdf", "jspdf-autotable"].map((name) => [name, packageVersion(name)]),
);
const fonts = Object.fromEntries(
  Object.entries(FONTS).map(([name, path]) => [
    name,
    createHash("sha256").update(readFileSync(path)).digest("hex"),
  ]),
);
writeFileSync(
  resolve(out, "receipt.json"),
  JSON.stringify(
    {
      engine: "jspdf",
      packages,
      fonts,
      node: process.version,
      platform: process.platform,
      arch: process.arch,
      runs,
      warmup: 10,
      results,
    },
    null,
    2,
  ),
);
await engine.dispose();

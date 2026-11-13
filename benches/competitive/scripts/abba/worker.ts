import { createRequire } from "node:module";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createHash } from "node:crypto";
import { resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { getWorkload } from "../../src/workloads/index.ts";
import { bodyHtml, layoutCss } from "../../src/workloads/html.ts";
import { FONTS, FONT_FAMILY } from "../../src/workloads/fonts.ts";
import { assertWorkloadText } from "../../src/compare/content.ts";
import { summarize } from "../../src/runner/stats.ts";

const [addonArg, outArg, runsArg = "100", idsArg = "report-100"] = process.argv.slice(2);
if (!addonArg || !outArg) throw new Error("usage: worker.ts ADDON OUT RUNS IDS");
const addon = resolve(addonArg);
const out = resolve(outArg);
const runs = Number(runsArg);
if (!Number.isInteger(runs) || runs < 0) throw new Error("invalid runs");
mkdirSync(out, { recursive: true });
const native = createRequire(import.meta.url)(addon);
if (native.version() !== "0.8.4") throw new Error("unexpected native version");
const fontFaces = [
  { data: readFileSync(FONTS.regular), family: FONT_FAMILY, weight: 400 },
  { data: readFileSync(FONTS.bold), family: FONT_FAMILY, weight: 700 },
];
const fontHandle = native.Fonts.load(fontFaces);
const results = [];
for (const id of idsArg.split(",")) {
  const workload = getWorkload(id as never);
  const html = bodyHtml(workload);
  const css = layoutCss(workload);
  const program = native.compile(html);
  const preflights = [];
  // Validation is outside all timing loops. Each serialization mode exercises
  // both fresh and reused program/font paths; keep the PDFs as durable goldens.
  for (const pdfUa of [false, true]) {
    for (const mode of ["cold", "warm"]) {
      const rendered =
        mode === "cold"
          ? native.compile(html).render({ css, now: 0, fonts: fontFaces, pdfUa })
          : program.render({ css, now: 0, pdfUa }, fontHandle);
      const name = `${id}-${pdfUa ? "tagged" : "plain"}-${mode}.pdf`;
      const path = resolve(out, name);
      writeFileSync(path, rendered.pdf);
      const text = execFileSync("pdftotext", ["-enc", "UTF-8", path, "-"], {
        encoding: "utf8",
        timeout: 30000,
      });
      assertWorkloadText(text, workload);
      const info = execFileSync("pdfinfo", [path], { encoding: "utf8", timeout: 30000 });
      const pages = Number(info.match(/^Pages:\s+(\d+)/m)?.[1]);
      if (pages !== rendered.pageCount) throw new Error(`page count mismatch ${name}`);
      if (pdfUa && !/^Tagged:\s+yes/m.test(info)) throw new Error(`missing tagging ${name}`);
      preflights.push({
        name,
        pages,
        bytes: rendered.pdf.length,
        sha256: createHash("sha256").update(rendered.pdf).digest("hex"),
      });
    }
  }
  const modes: Record<string, unknown> = {};
  if (runs > 0) {
    for (const mode of ["warm", "cold"]) {
      const render =
        mode === "cold"
          ? () => native.compile(html).render({ css, now: 0, fonts: fontFaces })
          : () => program.render({ css, now: 0 }, fontHandle);
      for (let i = 0; i < 10; i++) render();
      const samples = [];
      for (let i = 0; i < runs; i++) {
        const start = performance.now();
        const rendered = render();
        const elapsed = performance.now() - start;
        if (!rendered.pdf.length) throw new Error("empty timed PDF");
        samples.push(elapsed);
      }
      modes[mode] = { summary: summarize(samples), samples };
    }
  }
  results.push({ id, preflights, modes });
}
writeFileSync(
  resolve(out, "receipt.json"),
  JSON.stringify(
    {
      addon,
      addonSha256: createHash("sha256").update(readFileSync(addon)).digest("hex"),
      fonts: Object.fromEntries(
        Object.entries(FONTS).map(([name, path]) => [
          name,
          createHash("sha256").update(readFileSync(path)).digest("hex"),
        ]),
      ),
      version: native.version(),
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

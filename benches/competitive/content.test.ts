import assert from "node:assert/strict";
import { test } from "node:test";
import { spawnSync } from "node:child_process";
import { assertWorkloadPdf, assertWorkloadText } from "./src/compare/content.ts";
import { JspdfAdapter } from "./src/adapters/jspdf.ts";
import { getWorkload } from "./src/workloads/index.ts";
import { measure } from "./src/runner/measure.ts";
import type { EngineAdapter } from "./src/types.ts";

const legal = getWorkload("legal");
const fullText = [
  legal.content.title,
  legal.content.subtitle,
  ...legal.content.paragraphs,
  ...legal.content.footnotes,
].join("\n\f");

test("content admission includes prose and footnotes across page boundaries", () => {
  assertWorkloadText(fullText, legal);
  assert.throws(
    () => assertWorkloadText(legal.content.title + legal.content.subtitle, legal),
    /incomplete PDF content/,
  );
});

test("content admission counts repeated text instead of accepting the first paragraph", () => {
  const missingParagraph = fullText.replace(legal.content.paragraphs[20]!, "");
  assert.throws(() => assertWorkloadText(missingParagraph, legal), /incomplete PDF content/);
  const missingNote = fullText.replace(legal.content.footnotes.at(-1)!, "");
  assert.throws(() => assertWorkloadText(missingNote, legal), /incomplete PDF content/);
});

test("content admission rejects an omitted table row", () => {
  const report = getWorkload("report-100");
  const rowText = report.content.rows.map(
    (r) => `${r.id} ${r.description} ${r.qty} ${r.unit.toFixed(2)} ${r.amount.toFixed(2)}`,
  );
  const headings = `${report.content.title} ${report.content.subtitle}`;
  assertWorkloadText(`${headings} ${rowText.join(" ")}`, report);
  assert.throws(
    () => assertWorkloadText(`${headings} ${rowText.slice(1).join(" ")}`, report),
    /incomplete PDF content/,
  );
});

const extractor = spawnSync("pdftotext", ["-v"]).status === 0;
for (const [id, pages] of [
  ["legal", 8],
  ["mixed", 3],
  ["report-100", 3],
] as const) {
  test(`jsPDF admits the complete ${id} workload`, { skip: !extractor }, async (t) => {
    const adapter = new JspdfAdapter();
    if (!(await adapter.detect()).available) return t.skip("optional jsPDF is unavailable");
    const workload = getWorkload(id);
    const pdf = (await adapter.renderWarm(workload)).pdfBytes;
    const pageObjects =
      Buffer.from(pdf)
        .toString("latin1")
        .match(/\/Type\s*\/Page\b/g) ?? [];
    assert.equal(pageObjects.length, pages);
    await assertWorkloadPdf(pdf, workload);
    await adapter.dispose();
  });
}

test(
  "incomplete PDF cannot produce measured latency or throughput",
  { skip: !extractor },
  async (t) => {
    const adapter = new JspdfAdapter();
    if (!(await adapter.detect()).available) return t.skip("optional jsPDF is unavailable");
    const headingOnly = { ...legal, content: { ...legal.content, paragraphs: [], footnotes: [] } };
    const pdfBytes = (await adapter.renderWarm(headingOnly)).pdfBytes;
    let timedCalls = 0;
    const incomplete = {
      renderWarm: async () => ({ pdfBytes, timings: { renderMs: 0 } }),
      renderCold: async () => {
        timedCalls++;
        return { pdfBytes, timings: { renderMs: 0 } };
      },
    } as unknown as EngineAdapter;
    await assert.rejects(
      measure(incomplete, legal, { warmup: 0, runs: 1 }),
      /incomplete PDF content/,
    );
    assert.equal(timedCalls, 0);
    await adapter.dispose();
  },
);

test(
  "cold-only content omission cannot claim a fast cold render",
  { skip: !extractor },
  async (t) => {
    const adapter = new JspdfAdapter();
    if (!(await adapter.detect()).available) return t.skip("optional jsPDF is unavailable");
    const full = (await adapter.renderWarm(legal)).pdfBytes;
    const headingOnly = { ...legal, content: { ...legal.content, paragraphs: [], footnotes: [] } };
    const partial = (await adapter.renderWarm(headingOnly)).pdfBytes;
    const calls = { cold: 0, warm: 0 };
    const incomplete = {
      renderWarm: async () => {
        calls.warm++;
        return { pdfBytes: full, timings: { renderMs: 1 } };
      },
      renderCold: async () => {
        calls.cold++;
        return { pdfBytes: partial, timings: { renderMs: 0 } };
      },
    } as unknown as EngineAdapter;
    await assert.rejects(
      measure(incomplete, legal, { warmup: 0, runs: 1 }),
      /incomplete PDF content/,
    );
    assert.deepEqual(calls, { cold: 1, warm: 1 });
    await adapter.dispose();
  },
);

// Deterministic content generator. Seeded so every engine renders byte-identical
// logical content and reruns are reproducible (spec §0 determinism). No Date.now /
// Math.random anywhere — a small xorshift PRNG drives all "randomness".

import type { RowData, WorkloadContent } from "../types.ts";

/** Fixed seed. Changing this changes every generated corpus — keep pinned. */
const SEED = 0x9e3779b9;

/** Fixed reference date so headers/footers never drift between runs. */
export const FIXED_DATE = "2026-01-15";

/** Minimal deterministic xorshift32 PRNG. */
function makePrng(seed: number): () => number {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state / 0xffffffff;
  };
}

const WORDS = [
  "consulting",
  "license",
  "support",
  "hosting",
  "migration",
  "audit",
  "onboarding",
  "integration",
  "training",
  "retainer",
  "maintenance",
  "review",
];

function pick(rng: () => number, list: readonly string[]): string {
  const idx = Math.floor(rng() * list.length);
  return list[idx] ?? list[0] ?? "";
}

function makeRow(rng: () => number, id: number): RowData {
  const qty = 1 + Math.floor(rng() * 12);
  const unit = Math.round((10 + rng() * 990) * 100) / 100;
  const amount = Math.round(qty * unit * 100) / 100;
  const description = `${pick(rng, WORDS)} ${pick(rng, WORDS)} #${id}`;
  return { id, description, qty, unit, amount };
}

function buildRows(rng: () => number, count: number): RowData[] {
  const rows: RowData[] = [];
  for (let i = 1; i <= count; i++) {
    rows.push(makeRow(rng, i));
  }
  return rows;
}

const LOREM =
  "The parties hereto agree that the foregoing provisions shall be construed " +
  "in accordance with the governing law and shall remain in full force " +
  "notwithstanding any partial invalidity of the remaining clauses herein.";

function buildParagraphs(rng: () => number, count: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    const reps = 2 + Math.floor(rng() * 4);
    out.push(`§${i + 1}. ${LOREM.repeat(reps)}`);
  }
  return out;
}

function buildFootnotes(count: number): string[] {
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    out.push(`See clause ${i + 1}; cf. statutory schedule, ¶${i + 7}.`);
  }
  return out;
}

function sumAmounts(rows: readonly RowData[]): number {
  let total = 0;
  for (const r of rows) total += r.amount;
  return Math.round(total * 100) / 100;
}

/** Generate the content for a workload from its row/paragraph/footnote counts. */
export function generateContent(spec: {
  title: string;
  subtitle: string;
  rows: number;
  paragraphs: number;
  footnotes: number;
}): WorkloadContent {
  const rng = makePrng(SEED);
  const rows = buildRows(rng, spec.rows);
  const paragraphs = buildParagraphs(rng, spec.paragraphs);
  const footnotes = buildFootnotes(spec.footnotes);
  return {
    title: spec.title,
    subtitle: spec.subtitle,
    currency: "USD",
    rows,
    paragraphs,
    footnotes,
    total: sumAmounts(rows),
  };
}

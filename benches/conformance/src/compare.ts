// Compare candidate (engine) vs oracle (Chromium) box geometry per `data-cid`.
//
// For each cid the delta is max(|dx|, |dy|, |dw|, |dh|) — the worst single-axis
// disagreement, in px. A box passes when that stays within the tolerance. A cid
// present on only one side is a hard FAIL (the box was dropped or duplicated).

import type { Box } from "./types.ts";

export interface Row {
  cid: string;
  engine: Box | null;
  oracle: Box | null;
  delta: number | null;
  pass: boolean;
  note: string;
}

export interface FixtureResult {
  fixture: string;
  rows: Row[];
  passed: number;
  total: number;
}

function delta(a: Box, b: Box): number {
  return Math.max(
    Math.abs(a.x - b.x),
    Math.abs(a.y - b.y),
    Math.abs(a.width - b.width),
    Math.abs(a.height - b.height),
  );
}

/** Compare one cid's box across the two sides (either may be absent). */
function rowFor(cid: string, e: Box | null, o: Box | null, tolerance: number): Row {
  if (!e || !o) {
    const note = e ? "missing in chromium" : "missing in engine";
    return { cid, engine: e, oracle: o, delta: null, pass: false, note };
  }
  const d = delta(e, o);
  return { cid, engine: e, oracle: o, delta: d, pass: d <= tolerance, note: "" };
}

/** Build the per-cid comparison for one fixture. Union of cids from both sides
 *  so a box missing on either side surfaces. */
export function compareFixture(
  fixture: string,
  engine: Map<string, Box>,
  oracle: Map<string, Box>,
  tolerance: number,
): FixtureResult {
  const cids = [...new Set([...engine.keys(), ...oracle.keys()])].sort();
  const rows = cids.map((cid) =>
    rowFor(cid, engine.get(cid) ?? null, oracle.get(cid) ?? null, tolerance),
  );
  const passed = rows.filter((r) => r.pass).length;
  return { fixture, rows, passed, total: cids.length };
}

function n(v: number): string {
  return (Math.round(v * 10) / 10).toString();
}

function rect(b: Box | null): string {
  if (!b) return "—".padEnd(24);
  return `(${n(b.x)}, ${n(b.y)}, ${n(b.width)}, ${n(b.height)})`.padEnd(24);
}

/** Render one fixture's comparison as a text table. */
export function formatFixture(res: FixtureResult): string {
  const lines: string[] = [];
  const rate = `${res.passed}/${res.total}`;
  lines.push(`\n${res.fixture}  [${rate} within tolerance]`);
  lines.push(
    `  ${"cid".padEnd(10)}${"engine (x,y,w,h)".padEnd(24)}${"chromium (x,y,w,h)".padEnd(24)}${"Δ".padEnd(8)}result`,
  );
  for (const r of res.rows) {
    const d = r.delta === null ? "—" : `${n(r.delta)}px`;
    const status = r.pass ? "PASS" : `FAIL${r.note ? ` (${r.note})` : ""}`;
    lines.push(`  ${r.cid.padEnd(10)}${rect(r.engine)}${rect(r.oracle)}${d.padEnd(8)}${status}`);
  }
  return lines.join("\n");
}

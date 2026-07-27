// Layout-conformance harness (slice 1).
//
// For each fixture it lays the page out with turbo-html2pdf-core (via the napi
// `layoutBoxes` debug export) and with Chromium (via Playwright), pins both to
// the engine's bundled Inter face, then compares the border-box geometry of
// every `data-cid`-tagged element. Prints a per-fixture delta table and a
// summary; exits nonzero if any box is out of tolerance.
//
// Chromium is the ORACLE and may be absent: when Playwright/Chromium can't
// launch the harness prints "SKIPPED: no chromium" and exits 0 (never a red
// build for a missing browser). A missing/stale napi addon is a hard error
// (exit 1) — build it first with `pnpm build:addon`.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compareFixture, formatFixture, type FixtureResult } from "./compare.ts";
import { engineAvailable, engineBoxes } from "./engine.ts";
import { Oracle } from "./oracle.ts";
import { injectPin, pinStyle } from "./pin.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE_DIR = resolve(HERE, "../fixtures");

interface Args {
  width: number;
  height: number;
  tolerance: number;
  filter: string | null;
}

function parseArgs(argv: string[]): Args {
  const args: Args = { width: 800, height: 600, tolerance: 2, filter: null };
  // Flag -> setter, so the loop has a single branch (keeps cyclomatic cost low).
  const set: Record<string, (v: string) => void> = {
    "--width": (v) => {
      args.width = Number(v);
    },
    "--height": (v) => {
      args.height = Number(v);
    },
    "--tolerance": (v) => {
      args.tolerance = Number(v);
    },
    "--fixture": (v) => {
      args.filter = v;
    },
  };
  for (let i = 0; i < argv.length; i++) {
    const handler = set[argv[i] ?? ""];
    if (handler) handler(argv[++i] ?? "");
  }
  return args;
}

function listFixtures(filter: string | null): string[] {
  return readdirSync(FIXTURE_DIR)
    .filter((f) => f.endsWith(".html"))
    .filter((f) => !filter || f.includes(filter))
    .sort();
}

// A fixture that documents a known-hard, still-open gap carries a
// `<!-- conformance:defer <reason> -->` marker. Its box deltas are reported but do
// not gate the suite — we track the gap honestly instead of hacking the engine to
// force it green. Returns the trimmed reason, or null for a normally-gated fixture.
function deferralReason(raw: string): string | null {
  const m = raw.match(/conformance:defer\s+([\s\S]*?)-->/);
  return m ? m[1].replace(/\s+/g, " ").trim() : null;
}

async function main(): Promise<number> {
  const args = parseArgs(process.argv.slice(2));

  const eng = engineAvailable();
  if (!eng.ok) {
    console.error(`ERROR: ${eng.reason}`);
    return 1;
  }

  const { oracle, reason } = await Oracle.launch();
  if (!oracle) {
    console.log(`SKIPPED: no chromium (${reason}). Engine box-dump is fine; the`);
    console.log("oracle is unavailable, so there is nothing to compare against.");
    return 0;
  }

  const style = pinStyle();
  const fixtures = listFixtures(args.filter);
  console.log(
    `conformance: ${fixtures.length} fixtures @ ${args.width}x${args.height}, tolerance ${args.tolerance}px\n` +
      "font pinned to bundled Inter (Regular+Bold) on both sides",
  );

  const results: FixtureResult[] = [];
  try {
    for (const file of fixtures) {
      const raw = readFileSync(join(FIXTURE_DIR, file), "utf8");
      const html = injectPin(raw, style);
      const engine = engineBoxes(html, args.width, args.height);
      const chromium = await oracle.measure(html, args.width, args.height);
      const res = compareFixture(file, engine, chromium, args.tolerance, deferralReason(raw));
      results.push(res);
      console.log(formatFixture(res));
    }
  } finally {
    await oracle.close();
  }

  // The gate counts only NON-deferred fixtures; deferred ones are reported apart so
  // a documented gap never reds the build (but also never hides silently).
  const gated = results.filter((r) => !r.deferred);
  const deferred = results.filter((r) => r.deferred);
  const passed = gated.reduce((s, r) => s + r.passed, 0);
  const total = gated.reduce((s, r) => s + r.total, 0);
  const cleanFixtures = gated.filter((r) => r.passed === r.total).length;
  console.log(
    `\nsummary: ${passed}/${total} boxes within tolerance across ${gated.length} gated fixtures ` +
      `(${cleanFixtures} fixtures fully clean)`,
  );
  if (deferred.length > 0) {
    console.log(`deferred (tracked, not gated): ${deferred.length} fixture(s)`);
    for (const r of deferred)
      console.log(`  - ${r.fixture} [${r.passed}/${r.total}] — ${r.deferred}`);
  }
  return passed === total ? 0 : 1;
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(err);
    process.exit(1);
  });

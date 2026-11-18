import { spawnSync, execFileSync } from "node:child_process";
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
const [addonArg, outArg, ids = "invoice,report-100,legal,mixed", runs = "100"] =
  process.argv.slice(2);
if (!addonArg || !outArg) throw new Error("usage: fair-paired.mjs ADDON OUT IDS RUNS");
const out = resolve(outArg);
mkdirSync(out, { recursive: true });
const blocks = [];
const load = () => ({
  at: new Date().toISOString(),
  top: execFileSync("ps", ["-A", "-o", "pid=,pcpu=,comm="], { encoding: "utf8" })
    .trim()
    .split("\n")
    .sort((a, b) => Number(b.trim().split(/\s+/)[1]) - Number(a.trim().split(/\s+/)[1]))
    .slice(0, 12),
});
for (let cycle = 1; cycle <= 3; cycle++) {
  for (const [position, engine] of ["turbo", "jspdf", "jspdf", "turbo"].entries()) {
    const repeat = (cycle - 1) * 2 + (position < 2 ? 1 : 2);
    const dir = resolve(out, `${cycle}-${position + 1}-${engine}`);
    const loadBefore = load();
    const args =
      engine === "turbo"
        ? [resolve(here, "worker.ts"), resolve(addonArg), dir, runs, ids]
        : [resolve(here, "jspdf-worker.ts"), dir, runs, ids];
    const child = spawnSync(process.execPath, args, {
      env: { ...process.env, CARGO_NET_OFFLINE: "true", RUSTUP_TOOLCHAIN: "1.98.0" },
      encoding: "utf8",
      timeout: 300000,
    });
    writeFileSync(
      resolve(out, `${repeat}-${engine}.log`),
      `${child.stdout ?? ""}${child.stderr ?? ""}`,
    );
    if (child.status !== 0)
      throw new Error(`failed ${repeat}-${engine}: ${child.error ?? child.stderr}`);
    blocks.push({
      cycle,
      position: position + 1,
      repeat,
      engine,
      loadBefore,
      loadAfter: load(),
      receipt: JSON.parse(readFileSync(resolve(dir, "receipt.json"), "utf8")),
    });
    if (blocks.length % 2 === 0) {
      const [a, b] = blocks.slice(-2).map((block) => block.receipt);
      if (JSON.stringify(a.fonts) !== JSON.stringify(b.fonts))
        throw new Error("cross-engine font mismatch");
      if (
        a.results.map((result) => result.id).join(",") !==
        b.results.map((result) => result.id).join(",")
      )
        throw new Error("cross-engine workload mismatch");
    }
  }
}
writeFileSync(
  resolve(out, "fair-paired.json"),
  JSON.stringify(
    {
      ids,
      runs: Number(runs),
      cycles: 3,
      order: "turbo,jspdf,jspdf,turbo",
      blocks,
      notes:
        "Three complete ABBA cycles. Equivalent full logical content/fonts; HTML/CSS vs draw API, not pixel-identical. Cold includes per-document setup, not process startup. No RSS or parallel throughput claims.",
    },
    null,
    2,
  ),
);

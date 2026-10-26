// The turbo-dom-layout crate directory — its fonts (`assets/fonts`, the bundled faces
// and the test fixtures), notices and fixtures — located through `cargo metadata`, so
// a path, git or registry dependency all resolve. This repo keeps no copy of those
// files (turbo-dom consolidation, no vendoring); scripts, the napi tests and the JS
// benches read them from the crate.
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
let cached;

/** The turbo-dom-layout crate root (the directory of its Cargo.toml). */
export function layoutCrateDir() {
  if (cached === undefined) {
    const metadata = JSON.parse(
      execFileSync("cargo", ["metadata", "--format-version", "1"], {
        cwd: repoRoot,
        encoding: "utf8",
        maxBuffer: 256 * 1024 * 1024,
      }),
    );
    const layout = metadata.packages.find((pkg) => pkg.name === "turbo-dom-layout");
    if (!layout) throw new Error("turbo-dom-layout is not in this workspace's dependency graph");
    cached = dirname(layout.manifest_path);
  }
  return cached;
}

/** turbo-dom-layout's `assets/fonts` directory. */
export function layoutFontsDir() {
  return join(layoutCrateDir(), "assets", "fonts");
}

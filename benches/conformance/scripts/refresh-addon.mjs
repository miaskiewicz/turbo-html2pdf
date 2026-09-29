// Copy the freshly cargo-built napi cdylib next to the napi `index.js` under
// BOTH names its loader looks for — the platform-suffixed
// `turbo-pdf-napi.<platform>.node` (preferred) and the unsuffixed fallback — so
// the conformance harness always picks up the current `layoutBoxes` build rather
// than a stale prebuilt. Mirrors the suffix scheme in `crates/turbo-pdf-napi/index.js`.

import { copyFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", ".."); // benches/conformance/scripts -> repo root
const napiDir = join(repoRoot, "crates", "turbo-pdf-napi");

const { platform, arch } = process;
const dylib =
  platform === "darwin"
    ? "libturbo_pdf_napi.dylib"
    : platform === "win32"
      ? "turbo_pdf_napi.dll"
      : "libturbo_pdf_napi.so";

function suffix() {
  if (platform === "darwin") return `darwin-${arch}`;
  if (platform === "win32") return `win32-${arch}-msvc`;
  if (platform === "linux") return `linux-${arch}-gnu`;
  return `${platform}-${arch}`;
}

const src = join(repoRoot, "target", "release", dylib);
if (!existsSync(src)) {
  console.error(
    `refresh-addon: ${src} not found — run \`cargo build -p turbo-pdf-napi --release\` first`,
  );
  process.exit(1);
}
for (const name of [`turbo-pdf-napi.${suffix()}.node`, "turbo-pdf-napi.node"]) {
  const dest = join(napiDir, name);
  copyFileSync(src, dest);
  console.log(`refresh-addon: ${dylib} -> ${dest}`);
}

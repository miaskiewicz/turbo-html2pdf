#!/usr/bin/env bash
set -euo pipefail

arch="${1:?usage: build.sh x64|arm64}"
case "$arch" in
  x64) cpu=x86_64; machine="Advanced Micro Devices X86-64" ;;
  arm64) cpu=aarch64; machine=AArch64 ;;
  *) echo "Unsupported addon architecture: $arch" >&2; exit 2 ;;
esac
test "$(uname -m)" = "$cpu"
test "$(getconf GNU_LIBC_VERSION)" = "glibc 2.34"
gcc --version
rustc --version

checkout=$(mktemp -d)
cp -a /source/. "$checkout"
cd "$checkout"
cargo fetch --locked
export CARGO_NET_OFFLINE=true
layout_dir=$(cargo metadata --locked --format-version 1 | python3 -c '
import json, pathlib, sys
packages = json.load(sys.stdin)["packages"]
layout = next(p for p in packages if p["name"] == "turbo-dom-layout")
print(pathlib.Path(layout["manifest_path"]).parent)
')
package_dir=crates/turbo-pdf-napi
cp "$layout_dir/assets/fonts/Go-Regular.ttf" /output/Go-Regular.ttf
python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["version"])' \
  "$package_dir/package.json" > /output/VERSION

target="$cpu-unknown-linux-gnu"
addon="turbo-pdf-napi.linux-$arch-gnu.node"
for flavor in base svg; do
  flags=(--locked --release -p turbo-pdf-napi --target "$target")
  if [[ "$flavor" == svg ]]; then flags+=(--features svg); fi
  cargo build "${flags[@]}"
  output="/output/$flavor"
  mkdir -p "$output"
  cp "target/$target/release/libturbo_pdf_napi.so" "$output/$addon"
  cp "$package_dir/index.js" "$output/index.js"
  readelf --file-header --dynamic --version-info "$output/$addon" > "$output/elf.txt"
  python3 scripts/al2023/check-elf.py "$output/elf.txt" "$machine"
  (cd "$output" && sha256sum "$addon" > sha256.txt)
done

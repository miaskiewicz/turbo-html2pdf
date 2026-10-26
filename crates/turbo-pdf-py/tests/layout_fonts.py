"""turbo-dom-layout's font directory (``assets/fonts``), located through ``cargo
metadata`` so a path, git or registry dependency all resolve. This repo keeps no copy
of the fonts (turbo-dom consolidation, no vendoring); see scripts/layout-crate-dir.mjs.
"""

import json
import subprocess
from functools import lru_cache
from pathlib import Path

# crates/turbo-pdf-py/tests/ -> repo root is three parents up.
REPO_ROOT = Path(__file__).resolve().parents[3]


@lru_cache(maxsize=1)
def layout_fonts_dir() -> Path:
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    layout = next(p for p in metadata["packages"] if p["name"] == "turbo-dom-layout")
    return Path(layout["manifest_path"]).parent / "assets" / "fonts"

"""Reject addons whose ELF requirements exceed Amazon Linux 2023."""

import re
import sys
from pathlib import Path


def check_elf(report: str, expected_machine: str) -> None:
    machine = re.search(r"^\s*Machine:\s*(.+)$", report, re.MULTILINE)
    if not machine or machine.group(1) != expected_machine:
        raise ValueError(f"Expected ELF machine {expected_machine}")
    if not re.search(r"^\s*Class:\s*ELF64\s*$", report, re.MULTILINE):
        raise ValueError("Expected ELF64")
    if not re.search(r"^\s*Type:\s*DYN\b", report, re.MULTILINE):
        raise ValueError("Expected a shared object")
    if re.search(r"\((?:RPATH|RUNPATH)\)", report):
        raise ValueError("Unexpected embedded library search path")

    needs = report.partition("Version needs section")[2]
    versions = set(re.findall(r"\bName:\s+GLIBC_(\S+)", needs))
    if not versions:
        raise ValueError("No glibc version requirements found")
    for version in sorted(versions):
        if not re.fullmatch(r"\d+(?:\.\d+)+", version):
            raise ValueError(f"Unsupported glibc requirement GLIBC_{version}")
        if tuple(map(int, version.split("."))) > (2, 34):
            raise ValueError(f"glibc requirement GLIBC_{version} exceeds 2.34")


if __name__ == "__main__":
    check_elf(Path(sys.argv[1]).read_text(encoding="utf-8"), sys.argv[2])

"""Aggregate public dependency license texts, without including local paths."""

import os
from pathlib import Path
import re
import subprocess
import tomllib


root = Path(__file__).resolve().parents[1]
cargo = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
registries = list((cargo / "registry" / "src").glob("*"))
lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
groups: dict[str, list[str]] = {}
tree = subprocess.check_output([
    "cargo", "+1.99.1-x86_64-pc-windows-gnu", "tree", "--locked",
    "--target", "x86_64-pc-windows-gnu", "--prefix", "none",
    "--format", "{p}", "--edges", "normal,build",
], text=True)
selected = {match.groups() for line in tree.splitlines()
            if (match := re.match(r"^([\w-]+) v([^\s]+)", line))}

for package in lock["package"]:
    if "source" not in package or (package["name"], package["version"]) not in selected:
        continue
    name = f"{package['name']}-{package['version']}"
    sources = [directory / name for directory in registries if (directory / name).is_dir()]
    if len(sources) != 1:
        raise RuntimeError(f"Expected one downloaded source directory for {name}")
    files = [p for p in sources[0].iterdir() if p.is_file()]
    mit = [p for p in files if p.name.lower() in ("license-mit", "license-mit.txt")]
    if not mit:
        raise RuntimeError(f"Review licensing manually for {name}")
    notices = mit + [p for p in files if p.name.lower() == "license-unicode"]
    for path in notices:
        text = path.read_text(encoding="utf-8").replace("\r\n", "\n").strip()
        groups.setdefault(text, []).append(name)

sysroot = Path(subprocess.check_output(
    ["rustc", "+1.99.1-x86_64-pc-windows-gnu", "--print", "sysroot"], text=True
).strip())
for filename, component in [
    ("MIT.txt", "Rust standard library 1.99.1"),
    ("Apache-2.0.txt", "Rust / LLVM runtime support"),
    ("LLVM-exception.txt", "Rust / LLVM runtime support"),
    ("NCSA.txt", "Rust / LLVM runtime support"),
    ("GPL-3.0-or-later.txt", "GNU toolchain runtime support"),
    ("GCC-exception-3.1.txt", "GNU toolchain runtime support"),
]:
    path = sysroot / "share" / "doc" / "rust" / "licenses" / filename
    if not path.is_file():
        raise RuntimeError(f"Upstream runtime notice is missing: {filename}")
    groups.setdefault(path.read_text(encoding="utf-8").strip(), []).append(component)

parts = [
    "DeXDesk third-party notices",
    "MIT licensing is selected for dependencies offering MIT OR Apache-2.0.\n"
    "Build-time dependencies are included for completeness. Unicode data retains\n"
    "its additional notice. Windows system DLLs are not redistributed.",
]
for text, names in groups.items():
    parts.append("=" * 72 + "\n" + "\n".join(sorted(names)) + "\n\n" + text)
(root / "THIRD-PARTY-NOTICES.txt").write_text("\n\n".join(parts) + "\n", encoding="utf-8")

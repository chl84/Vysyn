#!/usr/bin/env python3
"""Bundle native codec dependencies; desktop/graphics libraries come from the OS."""
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

root = Path(__file__).resolve().parent.parent
bundle = root / "artifacts/vysyn-linux-x64"
lib = bundle / "lib"
lib.mkdir(parents=True, exist_ok=True)
binary = root / "target/release/vysyn"
shutil.copy2(binary, bundle / "vysyn-bin")
shutil.copy2(root / "target/release/vysyn-bench", bundle / "vysyn-bench-bin")
excluded = re.compile(r"^(linux-vdso|ld-linux|lib(c|m|dl|pthread|rt|resolv)\.so)")
pending = [binary]
seen = set()
while pending:
    item = pending.pop()
    result = subprocess.run(["ldd", str(item)], capture_output=True, text=True, check=True)
    for line in result.stdout.splitlines():
        if "not found" in line:
            raise SystemExit(f"Missing runtime dependency: {line}")
        match = re.match(r"\s*(\S+) => (/\S+)", line)
        if not match:
            continue
        name, path = match.groups()
        name = Path(name).name
        if excluded.match(name) or name in seen:
            continue
        seen.add(name)
        shutil.copy2(path, lib / name)
        pending.append(Path(path))
for name, binary_name in [("vysyn", "vysyn-bin"), ("vysyn-bench", "vysyn-bench-bin")]:
    launcher = bundle / name
    launcher.write_text('#!/bin/sh\nset -eu\nbase=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\n'
                        'export LD_LIBRARY_PATH="$base/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\n'
                        f'exec "$base/{binary_name}" "$@"\n')
    launcher.chmod(0o755)
shutil.copy2(root / "README.md", bundle / "README.md")
shutil.copytree(root / "docs", bundle / "docs", dirs_exist_ok=True)
shutil.copy2(root / "packaging/vysyn.desktop", bundle / "vysyn.desktop")
licenses = bundle / "third-party-licenses"
licenses.mkdir(exist_ok=True)
for package in ["libheif", "libde265", "dav1d", "aom", "x265", "x264", "openh264"]:
    for folder in [Path("/usr/share/licenses") / package, Path("/usr/share/doc") / package]:
        if folder.is_dir():
            for source in folder.rglob("*"):
                if source.is_file() and ("license" in source.name.lower() or "copying" in source.name.lower() or "copyright" in source.name.lower()):
                    shutil.copy2(source, licenses / f"{package}-{source.name}")
source_license = root / ".native/libheif/COPYING"
if source_license.exists():
    shutil.copy2(source_license, licenses / "libheif-COPYING")
with tarfile.open(root / "artifacts/vysyn-linux-x64.tar.gz", "w:gz") as archive:
    archive.add(bundle, arcname=bundle.name)
print(bundle)

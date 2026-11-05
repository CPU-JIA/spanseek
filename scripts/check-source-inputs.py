"""Reject literal Rust include inputs omitted from the Git source distribution."""
from pathlib import Path
import json
import re
import subprocess

ROOT = Path(__file__).resolve().parent.parent
tracked = set(subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode("utf-8").split("\0")) - {""}
literal = re.compile(r'include_(?:str|bytes)!\s*\(\s*"([^"\n]+)"\s*,?\s*\)', re.MULTILINE)
checked = 0
missing = []
for name in sorted(tracked):
    if not name.endswith(".rs"):
        continue
    source = ROOT / name
    for match in literal.finditer(source.read_text(encoding="utf-8")):
        target = (source.parent / match.group(1)).resolve()
        checked += 1
        relative = target.relative_to(ROOT).as_posix() if target.is_relative_to(ROOT) else None
        if relative not in tracked or not target.is_file():
            missing.append({"source": name, "input": match.group(1), "tracked": relative in tracked})
print(json.dumps({"literal_include_inputs": checked, "missing": missing}, ensure_ascii=False))
raise SystemExit(1 if missing else 0)

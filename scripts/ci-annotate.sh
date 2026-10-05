#!/usr/bin/env bash
# Turns compiler/test/bundler errors from log files into GitHub annotations,
# so failures are readable from the checks API without downloading logs.
# Usage: scripts/ci-annotate.sh LOG...
python3 - "$@" <<'PY'
import re, sys
blocks = []
for path in sys.argv[1:]:
    try:
        lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    except OSError:
        continue
    i = 0
    while i < len(lines):
        l = lines[i]
        if re.match(r"^(error(\[E\d+\])?:|thread '.*' panicked|\s*Error |failed to |error: )", l) or "panicked at" in l:
            blk = [l]
            j = i + 1
            while j < len(lines) and lines[j].strip() != "" and len(blk) < 18:
                blk.append(lines[j]); j += 1
            blocks.append(f"[{path}] " + "\n".join(blk))
            i = j
        else:
            i += 1
seen = []
for b in blocks:
    if b not in seen:
        seen.append(b)
def esc(s):
    return s.replace("%", "%25").replace("\r", "").replace("\n", "%0A")
# GitHub keeps ~10 error annotations per step: pack several errors into each.
for k in range(0, min(len(seen), 40), 4):
    chunk = "\n----\n".join(seen[k:k+4])[:7000]
    print(f"::error title=Build error {k//4+1}::{esc(chunk)}")
if not seen:
    print("::error title=Build failed::No recognisable error lines; see uploaded logs.")
PY

#!/usr/bin/env bash
# Fails if likely credentials appear in the repository or the built UI (A15).
set -euo pipefail
cd "$(dirname "$0")/.."
patterns='(sk-[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{30,}|xox[baprs]-[A-Za-z0-9-]{10,}|-----BEGIN (RSA |EC )?PRIVATE KEY-----|AIza[0-9A-Za-z_-]{35})'
targets=$(git ls-files | grep -v -E '^(docs/build-spec.md|scripts/check-secrets.sh)$' || true)
if [ -d apps/desktop/ui/dist ]; then targets="$targets $(find apps/desktop/ui/dist -type f)"; fi
if echo "$targets" | xargs grep -I -n -E "$patterns" 2>/dev/null; then
  echo "Possible secret found." >&2
  exit 1
fi
echo "No secrets found in tracked files or built UI."

#!/usr/bin/env bash
# Install the three programs from this checkout into a temporary root without network access, then probe the
# installed checker on a broken link and its fix. Exits 0 only when every probe behaves.
set -euo pipefail

checkout="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/mdlink-audit-smoke.XXXXXX")"
trap 'rm -rf "$work"' EXIT

cargo install --offline --locked --path "$checkout" --root "$work/root" --target-dir "$work/target"
for program in mdlink-audit mdlink-audit-action mdlink-audit-corpus; do
  if [ ! -x "$work/root/bin/$program" ]; then
    echo "smoke-install: $program was not installed" >&2
    exit 1
  fi
done
checker="$work/root/bin/mdlink-audit"

probe="$work/probe"
mkdir -p "$probe"
printf '# 安装\n' > "$probe/Guide.md"
printf '[guide](guide.md#安装)\n' > "$probe/README.md"
status=0
report="$("$checker" --root "$probe" --format json)" || status=$?
first_code="$(printf '%s\n' "$report" | grep -m1 '"code":' || true)"
if [ "$status" -ne 1 ] || [[ "$first_code" != *'"code": "case_mismatch"'* ]]; then
  echo "smoke-install: the installed checker failed the broken-link probe (exit $status)" >&2
  exit 1
fi

printf '[guide](Guide.md#安装)\n' > "$probe/README.md"
status=0
report="$("$checker" --root "$probe" --format json)" || status=$?
if [ "$status" -ne 0 ] || [[ "$report" != *'"links_checked": 1,'* ]]; then
  echo "smoke-install: the installed checker failed the fixed-link probe (exit $status)" >&2
  exit 1
fi

status=0
version="$("$checker" --version)" || status=$?
if [ "$status" -ne 0 ] || [ "$version" != "mdlink-audit 0.2.0" ]; then
  echo "smoke-install: --version printed '$version' (exit $status)" >&2
  exit 1
fi
echo "Install smoke test passed: offline install, three programs, broken and fixed links, version."

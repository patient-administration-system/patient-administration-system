#!/usr/bin/env bash
# Regenerate the status block in spec/14-implementation-status.md from the repository.
# Counts come from grep/find only (no compile, no network), so the output is deterministic.
#
#   scripts/generate-status.sh           rewrite the block
#   scripts/generate-status.sh --check   exit 1 if the block is stale (used by CI)
set -euo pipefail
cd "$(dirname "$0")/.."

FILE=spec/14-implementation-status.md
RUST=patient-administration-system-with-rust
FRONT=patient-administration-system-frontend
UI=patient-administration-system-ui-with-svelte

count() { grep -rE "$1" "${@:2}" 2>/dev/null | wc -l | tr -d ' '; }
files() { find "$@" 2>/dev/null | wc -l | tr -d ' '; }

rust_tests=$(count '^\s*#\[(tokio::)?test' $RUST/src $RUST/tests $RUST/migrations/src $FRONT/src $FRONT/tests)
rust_ignored=$(count '^\s*#\[ignore' $RUST/src $RUST/tests $FRONT/tests)
routes=$(count '^\s*\.route\(' $RUST/src/api)
migrations=$(files $RUST/migrations/src -name 'm2*.rs')
ui_pages=$(files $UI/src/routes -name '+page.svelte')
ui_unit=$(files $UI/tests/unit -name '*.test.ts')
ui_e2e=$(files $UI/tests/e2e -name '*.spec.ts')
locales=$(ls $UI/src/lib/messages 2>/dev/null | grep -c "\.json\$" || true)
tasks_done=$(grep -cE '^- \[x\]' tasks.md || true)
tasks_open=$(grep -cE '^- \[ \]' tasks.md || true)
frs=$(grep -cE '^\| FR-[0-9]+ ' spec/06-functional-requirements.md || true)
nfrs=$(grep -cE '^\| NFR-[0-9]+ ' spec/07-non-functional-requirements.md || true)
untested=$(cat spec/06-functional-requirements.md spec/07-non-functional-requirements.md | grep -ciE '\*\*Not (yet )?(tested|built)\*\*|\| Not (yet )?(tested|built)\b' || true)

# A row that says "not built (tasks.md X)" must cite at least one task that is still open;
# if every task it cites is ticked, the row is stale (QUAL-5).
python3 - <<'PY' || exit 1
import re, sys
tasks = open("tasks.md", encoding="utf-8").read()
ticked = set(re.findall(r"^- \[x\] \*\*([A-Z]+-\d+)", tasks, re.M))
opened = set(re.findall(r"^- \[ \] \*\*([A-Z]+-\d+)", tasks, re.M))
bad = []
for f in ("spec/06-functional-requirements.md", "spec/07-non-functional-requirements.md"):
    for line in open(f, encoding="utf-8"):
        m = re.match(r"\| ((?:N?FR)-\d+) \|.*\*\*Not yet built\*\*[^|]*tasks\.md ([^)]*)\)", line)
        if not m:
            continue
        ids = re.findall(r"[A-Z]+-\d+", m.group(2))
        if ids and not any(i in opened for i in ids) and all(i in ticked for i in ids):
            bad.append(f"{m.group(1)} cites only ticked tasks: {', '.join(ids)}")
if bad:
    print("stale 'not yet built' rows:\n  " + "\n  ".join(bad), file=sys.stderr)
    sys.exit(1)
PY

block=$(cat <<BLOCK
| Measure | Count |
|---|---|
| Rust test functions (server, front-end, migrations) | $rust_tests |
| of which \`#[ignore]\`d (need PostgreSQL) | $rust_ignored |
| Server route registrations | $routes |
| Migrations | $migrations |
| UI page routes | $ui_pages |
| UI unit test files | $ui_unit |
| UI browser spec files | $ui_e2e |
| UI locale catalogues | $locales |
| Functional requirements | $frs |
| Non-functional requirements | $nfrs |
| Requirements marked not yet tested or not yet built | $untested |
| Tasks done | $tasks_done |
| Tasks open | $tasks_open |
BLOCK
)

python3 - "$FILE" "$block" "${1:-}" <<'PY'
import sys
path, block, mode = sys.argv[1], sys.argv[2], sys.argv[3]
begin, end = "<!-- status:begin -->", "<!-- status:end -->"
text = open(path, encoding="utf-8").read()
a, b = text.index(begin) + len(begin), text.index(end)
new = text[:a] + "\n" + block + "\n" + text[b:]
if mode == "--check":
    if new != text:
        print(f"{path} is stale: run scripts/generate-status.sh", file=sys.stderr)
        sys.exit(1)
    print(f"{path} is current")
else:
    open(path, "w", encoding="utf-8").write(new)
    print(f"{path} updated")
PY

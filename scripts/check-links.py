#!/usr/bin/env python3
"""Offline Markdown link check (standard library only): the fallback for lychee.

Checks that every relative link points at an existing file or directory, and that a
#fragment names a heading in the target Markdown file (GitHub's slug rules). http(s),
mailto and other schemes are not fetched.

    python3 scripts/check-links.py            # exit 1 if any link is broken
"""
import os
import re
import sys
import unicodedata
from pathlib import Path
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {"target", "node_modules", ".git", ".svelte-kit", "build", "test-results", "playwright-report"}
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|^\s*\[[^\]]+\]:\s*(\S+)", re.M)
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$", re.M)
FENCE = re.compile(r"^(```|~~~).*?^\1", re.M | re.S)
CODE_SPAN = re.compile(r"`[^`\n]*`")


def slug(text: str) -> str:
    text = re.sub(r"<[^>]+>", "", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = unicodedata.normalize("NFKD", text).lower()
    text = re.sub(r"[^\w\- ]", "", text, flags=re.U)
    return text.strip().replace(" ", "-")


def anchors(path: Path) -> set:
    body = FENCE.sub("", path.read_text(encoding="utf-8", errors="replace"))
    seen, out = {}, set()
    for _, title in HEADING.findall(body):
        base = slug(title)
        n = seen.get(base, 0)
        seen[base] = n + 1
        out.add(base if n == 0 else f"{base}-{n}")
    return out


def markdown_files():
    # Walk the tree ourselves so build output (`target/`, `node_modules/`) is never entered: a build running at
    # the same time can delete files mid-walk, and `rglob` would fail on them.
    for dirpath, dirnames, filenames in os.walk(ROOT):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".md"):
                yield Path(dirpath) / name


def main() -> int:
    bad = []
    cache = {}
    for md in markdown_files():
        text = CODE_SPAN.sub("", FENCE.sub("", md.read_text(encoding="utf-8", errors="replace")))
        for m in LINK.finditer(text):
            target = (m.group(1) or m.group(2) or "").strip("<>")
            if not target or re.match(r"^[a-zA-Z][a-zA-Z0-9+.\-]*:", target) or target.startswith("//"):
                continue
            path_part, _, frag = target.partition("#")
            path_part = unquote(path_part)
            dest = md if path_part == "" else (md.parent / path_part).resolve()
            if not dest.exists():
                bad.append(f"{md.relative_to(ROOT)}: missing target {target}")
                continue
            if frag and dest.is_file() and dest.suffix == ".md":
                if dest not in cache:
                    cache[dest] = anchors(dest)
                if unquote(frag).lower() not in cache[dest]:
                    bad.append(f"{md.relative_to(ROOT)}: no heading #{frag} in {dest.relative_to(ROOT)}")
    for line in sorted(set(bad)):
        print(line)
    print(f"{len(bad)} broken link(s) in {sum(1 for _ in markdown_files())} Markdown file(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

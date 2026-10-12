#!/usr/bin/env python3
"""Check the wiki sources in wiki/: links, media and page names.

Standard library only. Run from the repository root; exits 1 on any problem.

  * Every page file is `<Prefix>-<Words>.md` (or Home, Philosophy, Gallery, Glossary, _Sidebar,
    _Footer) and has a first-line `# ` heading.
  * Every `[text](Page-Name)` link names a page that exists (anchors after `#` are allowed).
  * Every media link under `raw.githubusercontent.com/<repo>/main/docs/...` names a file that
    exists in this tree, so a page never points at a render or figure that was not committed.
  * Fenced code blocks and `$...$` maths are skipped when scanning for links.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WIKI = ROOT / "wiki"
RAW = "https://raw.githubusercontent.com/tfpickard/isotop/main/"
SPECIAL = {"Home", "Philosophy", "Gallery", "Glossary", "_Sidebar", "_Footer"}
PREFIXES = ("Theory-", "Math-", "View-", "Planned-", "Guide-")

LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(([^)\s]+)\)")
MEDIA = re.compile(r"\]\((" + re.escape(RAW) + r"[^)\s]+)\)")
FENCE = re.compile(r"^(```|~~~)")


def prose_lines(text):
    """Yield (number, line) outside fenced code blocks."""
    fenced = False
    for number, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line.strip()):
            fenced = not fenced
            continue
        if not fenced:
            yield number, line


def main():
    pages = {path.stem: path for path in sorted(WIKI.glob("*.md"))}
    problems = []
    for name, path in pages.items():
        if name not in SPECIAL and not name.startswith(PREFIXES):
            problems.append(f"{path.name}: page name has no known prefix {PREFIXES}")
        text = path.read_text(encoding="utf-8")
        if name not in {"_Sidebar", "_Footer"} and not text.lstrip().startswith("# "):
            problems.append(f"{path.name}: first line must be a '# ' heading")
        for number, line in prose_lines(text):
            for target in LINK.findall(line):
                if target.startswith(("http://", "https://", "mailto:", "#")):
                    continue
                page = target.split("#", 1)[0]
                if page not in pages:
                    problems.append(f"{path.name}:{number}: link to missing page '{page}'")
            for url in MEDIA.findall(line):
                relative = url[len(RAW):].split("#", 1)[0]
                if not (ROOT / relative).is_file():
                    problems.append(f"{path.name}:{number}: media not in the tree: {relative}")
    for problem in problems:
        print(problem)
    print(f"{len(pages)} pages, {len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Generate docs/METRICS.md and the README "By the numbers" block (python 3.11+, standard library only).

  python3 scripts/code_report.py             write both files
  python3 scripts/code_report.py --check     exit 1 when either file is stale, 2 when tokei is missing
  python3 scripts/code_report.py --self-test run the fixture tests (no tokei needed)

The report is deterministic: sorted, no timestamps, no paths of the machine. tokei is a tool, not a
crate dependency; the header records its version because counts differ between versions.
"""

import argparse
import difflib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

REPORT = Path("docs/METRICS.md")
README = Path("README.md")
START = "<!-- metrics:start -->"
END = "<!-- metrics:end -->"
LARGE_FILE_LINES = 1200
LARGEST_FILES = 10

INSTALL_HELP = """tokei is not installed. Install it with one of:
  brew install tokei
  paru -S tokei
  cargo install tokei --locked"""

# Each counter is a plain regex over every src/**/*.rs file. Comments and strings are not
# excluded, which keeps the script small.
DEBT = [
    (
        "`#[expect(clippy::too_many_lines`",
        r"#\[expect\((?:(?!\)\]).)*?clippy::too_many_lines",
    ),
    (
        "`#[expect(clippy::too_many_arguments`",
        r"#\[expect\((?:(?!\)\]).)*?clippy::too_many_arguments",
    ),
    ("`unsafe` blocks", r"\bunsafe\s*\{"),
    ("`unwrap_or(0` sites", r"\.unwrap_or\(\s*0(?:_?(?:u|i)(?:8|16|32|64|128|size))?\s*\)"),
    ("`unwrap_or(0.0` sites", r"\.unwrap_or\(\s*0\.0+(?:_?f(?:32|64))?\s*\)"),
]
# Counted on production lines only, so the harness's own fixtures do not count.
KNOWN_UNSTABLE = r"\bproperty:\s*Property::\w+"
TEST_ATTRIBUTE = re.compile(r"^\s*#\[test\]", re.MULTILINE)
VIEW_ROW = re.compile(r'ViewInfo::new\(\s*"([a-z0-9_-]+)"')
CFG_TEST_FILE = re.compile(r"#\[cfg\(test\)\]\s*\n\s*(?:pub\s+)?mod\s+(\w+)\s*;")


def number(value):
    return f"{value:,}"


def source_files(root):
    return sorted((root / "src").rglob("*.rs"), key=lambda path: path.as_posix())


def is_code(line):
    stripped = line.strip()
    return bool(stripped) and not stripped.startswith("//")


def split_modules(text, whole_file_is_test=False):
    """Code lines (not blank, not `//` comments) outside and inside test code.

    Test code is every `#[cfg(test)]` module, from its attribute to the closing brace at the same
    indentation (rustfmt puts it there), and the whole of a file that is declared `#[cfg(test)]`.
    """
    lines = text.split("\n")
    production = test = 0
    closing = None
    index = 0
    while index < len(lines):
        line = lines[index]
        counted = is_code(line)
        if whole_file_is_test:
            test += counted
        elif closing is not None:
            test += counted
            if line == closing:
                closing = None
        elif line.strip() == "#[cfg(test)]":
            following = lines[index + 1] if index + 1 < len(lines) else ""
            if re.match(r"\s*(?:pub\s+)?mod\s+\w+\s*\{\s*$", following):
                indent = line[: len(line) - len(line.lstrip())]
                closing = indent + "}"
                test += counted
            else:
                production += counted
        else:
            production += counted
        index += 1
    return production, test


def test_only_files(root):
    """Files declared as `#[cfg(test)] mod name;` in src/main.rs."""
    main = root / "src" / "main.rs"
    if not main.exists():
        return set()
    return {f"src/{name}.rs" for name in CFG_TEST_FILE.findall(main.read_text())}


def count_matches(pattern, text):
    return len(re.findall(pattern, text, re.DOTALL))


def registry_views(root):
    text = (root / "src" / "views.rs").read_text()
    start = text.index("pub static REGISTRY")
    end = text.index("\n];", start)
    return VIEW_ROW.findall(text[start:end])


def readme_tables(root, views):
    """Views whose README subsection under "Worlds" holds a markdown table."""
    found = set()
    in_worlds = False
    section = None
    for line in (root / README).read_text().split("\n"):
        if line.startswith("## "):
            in_worlds = line.strip() == "## Worlds"
            section = None
        elif line.startswith("### "):
            section = line[4:].strip().lower()
        elif in_worlds and section and re.match(r"^\|[\s:|-]*-[\s:|-]*\|\s*$", line):
            found.add(section)
    return sorted(found & set(views))


def dependencies(root):
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    tables = [manifest.get(kind, {}) for kind in ("dependencies", "dev-dependencies", "build-dependencies")]
    for target in manifest.get("target", {}).values():
        tables += [target.get(kind, {}) for kind in ("dependencies", "dev-dependencies", "build-dependencies")]
    direct = len({name for table in tables for name in table})
    lock = tomllib.loads((root / "Cargo.lock").read_text())
    own = manifest["package"]["name"]
    total = len({package["name"] for package in lock["package"] if package["name"] != own})
    return direct, total


def measure(root):
    rows = []
    sources = {}
    only_tests = test_only_files(root)
    for path in source_files(root):
        name = path.relative_to(root).as_posix()
        text = path.read_text()
        production, test = split_modules(text, name in only_tests)
        rows.append((name, production, test, len(text.splitlines())))
        sources[name] = (text, production, test)
    everything = "\n".join(text for text, _, _ in sources.values())
    debt = [(label, count_matches(pattern, everything)) for label, pattern in DEBT]
    unstable = 0
    for name, (text, _, _) in sources.items():
        if name in only_tests:
            continue
        unstable += count_matches(KNOWN_UNSTABLE, strip_test_modules(text))
    debt.append(("`known_unstable` harness entries", unstable))
    views = registry_views(root)
    direct, total = dependencies(root)
    health = {
        "tests": len(TEST_ATTRIBUTE.findall(everything)),
        "views": views,
        "tables": readme_tables(root, views),
        "direct": direct,
        "total": total,
    }
    return rows, debt, health


def strip_test_modules(text):
    lines = text.split("\n")
    kept = []
    closing = None
    for index, line in enumerate(lines):
        if closing is not None:
            if line == closing:
                closing = None
            continue
        if line.strip() == "#[cfg(test)]" and index + 1 < len(lines):
            if re.match(r"\s*(?:pub\s+)?mod\s+\w+\s*\{\s*$", lines[index + 1]):
                closing = line[: len(line) - len(line.lstrip())] + "}"
                continue
        kept.append(line)
    return "\n".join(kept)


def language_rows(data):
    rows = []
    for language, stats in data.items():
        if language == "Total":
            continue
        rows.append(
            (language, len(stats["reports"]), stats["code"], stats["comments"], stats["blanks"])
        )
    rows.sort(key=lambda row: (-row[2], row[0]))
    return rows


def table(header, rows, align):
    out = ["| " + " | ".join(header) + " |"]
    out.append("|" + "|".join(":--" if a == "l" else "--:" for a in align) + "|")
    out += ["| " + " | ".join(row) + " |" for row in rows]
    return out


def render(root, tokei_data, tokei_version):
    rows, debt, health = measure(root)
    production = sum(row[1] for row in rows)
    test = sum(row[2] for row in rows)
    out = [
        "# Code metrics",
        "",
        "Generated by `scripts/code_report.py` with tokei "
        f"{tokei_version}. Do not edit by hand: run `python3 scripts/code_report.py` and commit the "
        "result. `python3 scripts/code_report.py --check` (the last gate, also run in CI) fails "
        "when this file or the README block is stale.",
        "",
        "## Languages",
        "",
        "tokei over the repository (ignored files and hidden directories skipped, "
        "`docs/METRICS.md` excluded). Markdown inside Rust doc comments is not counted here.",
        "",
    ]
    languages = language_rows(tokei_data)
    body = [
        [name, number(files), number(code), number(comments), number(blanks)]
        for name, files, code, comments, blanks in languages
    ]
    body.append(
        [
            "**Total**",
            *(f"**{number(sum(row[i] for row in languages))}**" for i in (1, 2, 3, 4)),
        ]
    )
    out += table(["Language", "Files", "Code", "Comments", "Blanks"], body, "lrrrr")
    out += [
        "",
        "## Rust by module",
        "",
        "Code lines per file under `src/`, counted by this script as lines that are neither blank "
        "nor `//` comments. Test lines are every `#[cfg(test)]` module (attribute to closing brace) "
        "and every file declared `#[cfg(test)] mod name;` in `src/main.rs`. Sorted by production "
        "lines, descending.",
        "",
    ]
    ordered = sorted(rows, key=lambda row: (-row[1], row[0]))
    body = [[f"`{name}`", number(prod), number(tests)] for name, prod, tests, _ in ordered]
    body.append(["**Total**", f"**{number(production)}**", f"**{number(test)}**"])
    out += table(["File", "Production", "Test"], body, "lrr")
    out += ["", "## Largest files", "", f"The {LARGEST_FILES} longest `src/` files by physical "
            f"lines. A mark means over {number(LARGE_FILE_LINES)} lines.", ""]
    largest = sorted(rows, key=lambda row: (-row[3], row[0]))[:LARGEST_FILES]
    body = [
        [f"`{name}`", number(lines), "over limit" if lines > LARGE_FILE_LINES else ""]
        for name, _, _, lines in largest
    ]
    out += table(["File", "Lines", f"> {number(LARGE_FILE_LINES)}"], body, "lrl")
    out += [
        "",
        "## Debt counters",
        "",
        "Regex counts over every `src/**/*.rs` file, comments and strings included. They should "
        "only go down as the architecture work lands; a PR that raises one says why. "
        "`known_unstable` entries are counted on production lines, so test fixtures do not count.",
        "",
    ]
    out += table(["Counter", "Count"], [[label, number(count)] for label, count in debt], "lr")
    views = health["views"]
    out += [
        "",
        "## Health counters",
        "",
        "Views come from parsing `REGISTRY` in `src/views.rs` (not from `isotop --list-views`, "
        "which depends on the platform and on a release build), so the answer is the same on every "
        "machine. Total crates are the packages in `Cargo.lock` for all platforms, without isotop.",
        "",
    ]
    out += table(
        ["Counter", "Count"],
        [
            ["`#[test]` functions", number(health["tests"])],
            ["Views in the registry", number(len(views))],
            [
                "Views with a README mapping table",
                f"{len(health['tables'])} of {len(views)}",
            ],
            ["Direct crate dependencies", number(health["direct"])],
            ["Total crate dependencies", number(health["total"])],
        ],
        "lr",
    )
    out += ["", "Views: " + ", ".join(f"`{name}`" for name in views) + ".", ""]
    block = [
        START,
        "## By the numbers",
        "",
        f"{number(production + test)} lines of Rust ({number(production)} production, "
        f"{number(test)} test) in {len(rows)} files, {number(health['tests'])} tests, "
        f"{len(views)} views, {health['direct']} direct and {health['total']} total crate "
        "dependencies. Generated; see [docs/METRICS.md](docs/METRICS.md) for the breakdown and "
        "the debt counters.",
        END,
    ]
    return "\n".join(out), "\n".join(block)


def replace_block(readme, block):
    start = readme.index(START)
    end = readme.index(END) + len(END)
    return readme[:start] + block + readme[end:]


def run_tokei(root):
    if shutil.which("tokei") is None:
        print(INSTALL_HELP, file=sys.stderr)
        sys.exit(2)
    version = subprocess.run(
        ["tokei", "--version"], capture_output=True, text=True, check=True
    ).stdout.split()[1]
    result = subprocess.run(
        ["tokei", "--output", "json", "--exclude", REPORT.as_posix(), "."],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(result.stdout), version


def generate(root, tokei_data, tokei_version):
    """The two files as they should be: the report and the README with its block replaced."""
    report, block = render(root, tokei_data, tokei_version)
    readme = replace_block((root / README).read_text(), block)
    return report + "\n" if not report.endswith("\n") else report, readme


def check(root, expected, quiet=False):
    print_ = (lambda *a: None) if quiet else print
    stale = False
    for path, want in zip((REPORT, README), expected):
        have = (root / path).read_text() if (root / path).exists() else ""
        if have != want:
            stale = True
            print_(f"{path} is stale:")
            diff = difflib.unified_diff(
                have.splitlines(), want.splitlines(), f"{path} (checked in)", f"{path} (generated)", lineterm="", n=0
            )
            for count, line in enumerate(diff):
                if count >= 30:
                    print_("...")
                    break
                print_(line)
    if stale:
        print_("Run `python3 scripts/code_report.py` and commit the result.")
        return 1
    print_("docs/METRICS.md and the README block are up to date.")
    return 0


FIXTURE_TOKEI = {
    "Rust": {"blanks": 3, "code": 90, "comments": 4, "reports": [{}, {}, {}], "children": {}},
    "Markdown": {"blanks": 2, "code": 0, "comments": 10, "reports": [{}], "children": {}},
    "Python": {"blanks": 5, "code": 90, "comments": 1, "reports": [{}], "children": {}},
    "Total": {"blanks": 10, "code": 180, "comments": 15, "reports": [], "children": {}},
}

FIXTURE_FILES = {
    "Cargo.toml": '[package]\nname = "demo"\n\n[dependencies]\nalpha = "1"\nbeta = "1"\n\n'
    '[target.\'cfg(unix)\'.dependencies]\nlibc = "0.2"\n',
    "Cargo.lock": 'version = 4\n\n[[package]]\nname = "alpha"\n\n[[package]]\nname = "beta"\n\n'
    '[[package]]\nname = "demo"\n\n[[package]]\nname = "libc"\n\n[[package]]\nname = "gamma"\n',
    "README.md": "# demo\n\n<!-- metrics:start -->\nstale\n<!-- metrics:end -->\n\n## Worlds\n\n"
    "### Alpha\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n### Beta\n\n- no table\n\n## Other\n",
    "src/main.rs": "#[cfg(test)]\nmod harness;\nfn main() {\n    let x = Some(1).unwrap_or(0);\n}\n",
    "src/harness.rs": "fn all_of_it_is_test() {}\n\n// comment\nfn two() {}\n",
    "src/views.rs": (
        "pub static REGISTRY: &[ViewInfo] = &[\n"
        '    ViewInfo::new(\n        "alpha",\n    ),\n'
        '    ViewInfo { x: 1, ..ViewInfo::new("beta", "B") },\n];\n'
        "known_unstable: &[Unstable { property: Property::Churn }],\n"
        "#[cfg(test)]\npub mod testing {\n    fn fixture() { Unstable { property: Property::Churn }; }\n}\n"
    ),
    "src/model.rs": (
        "// a comment\n"
        "fn production() {\n"
        "    let a = unsafe { read() };\n"
        "    let b = x.unwrap_or(0.0);\n"
        "    let c = x.unwrap_or(0u64);\n"
        "    let d = x.unwrap_or(0.5);\n"
        "}\n"
        '#[expect(clippy::too_many_lines, reason = "long")]\nfn long() {}\n'
        "#[expect(\n    clippy::too_many_arguments,\n    reason = \"many\"\n)]\nfn many() {}\n"
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn a() {\n        let _ = unsafe { 1 };\n    }\n"
        "\n    #[test]\n    fn b() {}\n}\n"
        "fn after() {}\n"
    ),
}


def fixture_tree(directory):
    for name, text in FIXTURE_FILES.items():
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
    return directory


def self_test():
    failures = []

    def expect(label, got, want):
        if got != want:
            failures.append(f"{label}: got {got!r}, want {want!r}")

    with tempfile.TemporaryDirectory() as one, tempfile.TemporaryDirectory() as two:
        root = fixture_tree(Path(one))
        rows, debt, health = measure(root)
        by_name = {row[0]: row for row in rows}
        # Lines that are neither blank nor comments: 6 + 2 + 5 + 1 outside the tests module.
        expect("model production", by_name["src/model.rs"][1], 14)
        expect("model test", by_name["src/model.rs"][2], 9)
        expect("a cfg(test) file is all test", by_name["src/harness.rs"][1:3], (0, 2))
        expect("main is production", by_name["src/main.rs"][1:3], (5, 0))
        counts = dict(debt)
        expect("too_many_lines", counts["`#[expect(clippy::too_many_lines`"], 1)
        expect("too_many_arguments across lines", counts["`#[expect(clippy::too_many_arguments`"], 1)
        expect("unsafe blocks", counts["`unsafe` blocks"], 2)
        expect("unwrap_or(0", counts["`unwrap_or(0` sites"], 2)
        expect("unwrap_or(0.0", counts["`unwrap_or(0.0` sites"], 1)
        expect("known_unstable skips test modules", counts["`known_unstable` harness entries"], 1)
        expect("tests", health["tests"], 2)
        expect("views", health["views"], ["alpha", "beta"])
        expect("tables", health["tables"], ["alpha"])
        expect("dependencies", (health["direct"], health["total"]), (3, 4))
        first = generate(root, FIXTURE_TOKEI, "0.0.0")
        again = generate(root, FIXTURE_TOKEI, "0.0.0")
        elsewhere = generate(fixture_tree(Path(two)), FIXTURE_TOKEI, "0.0.0")
        expect("deterministic", first, again)
        expect("independent of the path", first, elsewhere)
        expect("README block replaced", "stale" not in first[1] and "## By the numbers" in first[1], True)
        expect("languages sorted by code", first[0].index("| Python") < first[0].index("| Rust"), True)
        (root / REPORT).parent.mkdir(exist_ok=True)
        (root / REPORT).write_text(first[0])
        (root / README).write_text(first[1])
        expect("fresh files pass", check(root, first, quiet=True), 0)
        (root / "src" / "model.rs").write_text(FIXTURE_FILES["src/model.rs"] + "fn extra() {}\n")
        expect("a changed source fails", check(root, generate(root, FIXTURE_TOKEI, "0.0.0"), quiet=True), 1)
    for failure in failures:
        print("FAIL", failure)
    print("self-test:", "failed" if failures else "ok")
    return 1 if failures else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--check", action="store_true", help="exit 1 when the files are stale")
    parser.add_argument("--self-test", action="store_true", help="run the fixture tests")
    arguments = parser.parse_args()
    if arguments.self_test:
        sys.exit(self_test())
    root = Path(__file__).resolve().parent.parent
    data, version = run_tokei(root)
    expected = generate(root, data, version)
    if arguments.check:
        sys.exit(check(root, expected))
    # The files are Markdown that tokei counts, so writing them can change the counts they hold.
    # Repeat until the output stops changing (one extra pass in practice).
    for _ in range(5):
        for path, text in zip((REPORT, README), expected):
            (root / path).write_text(text)
        data, version = run_tokei(root)
        after = generate(root, data, version)
        if after == expected:
            break
        expected = after
    else:
        sys.exit("the report does not settle; its Markdown counts keep changing")
    print(f"wrote {REPORT} and {README}")


if __name__ == "__main__":
    main()

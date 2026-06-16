#!/usr/bin/env python3
"""veyo docs build — render every Markdown doc to a styled, cross-linked HTML page.

Usage:
    python3 build.py            # build all docs/*.md -> docs/*.html
    python3 build.py --check    # fail if any .html is stale vs its .md

Single source of truth is the Markdown. HTML is generated; do not hand-edit it.
Requires: the `markdown` package (stdlib + `pip install markdown`).
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

try:
    import markdown
except ImportError:  # pragma: no cover
    sys.exit("error: the `markdown` package is required (pip install markdown)")

DOCS_DIR = Path(__file__).resolve().parent

# Ordered nav: (markdown stem, nav label, section group).
# `README` renders to both README.html and index.html (browser landing).
NAV: list[tuple[str, str, str]] = [
    ("README",                   "Overview / Index",      "Start here"),
    ("product-overview",         "Product Overview",      "The What & Why"),
    ("architecture",             "Architecture",          "The How"),
    ("tech-specs",               "Tech Specs",            "The How"),
    ("policy-engine",            "Policy Engine (IP)",    "The How"),
    ("privacy-model",            "Privacy Model",         "The How"),
    ("eval-harness",             "Eval Harness",          "Execution"),
    ("plan",                     "Build Plan",            "Execution"),
    ("phases",                   "Phases & Roadmap",      "Execution"),
    ("risks-and-open-questions", "Risks & Open Qs",       "Execution"),
    ("glossary",                 "Glossary",              "Reference"),
]

MD_EXTENSIONS = ["extra", "tables", "fenced_code", "toc", "attr_list", "sane_lists"]
MD_EXT_CONFIGS = {"toc": {"permalink": False, "toc_depth": "2-3"}}

PAGE = """<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="generator" content="veyo docs/build.py — generated from {stem}.md, do not edit">
<title>{title} · veyo docs</title>
<link rel="stylesheet" href="assets/style.css">
</head>
<body>
<div class="layout">
<aside class="sidebar">
  <a class="brand" href="index.html"><b>veyo</b><span>docs</span></a>
  <p class="tagline">A local-first visual event codec — continuous, affordable, private sight for an LLM.</p>
  <nav>
{nav}
  </nav>
</aside>
<div class="content">
<article class="article">
{toc}
{body}
<footer class="docfooter">
  <span>Generated from <code>{stem}.md</code> — edit the Markdown, then run <code>python3 build.py</code>.</span>
  <span>veyo · product spec v0.1 (draft)</span>
</footer>
</article>
</div>
</div>
</body>
</html>
"""


def build_nav(current_stem: str) -> str:
    """Sidebar nav HTML, grouping consecutive entries by their section label."""
    lines: list[str] = []
    last_group: str | None = None
    for stem, label, group in NAV:
        if group != last_group:
            lines.append(f'    <div class="grouplabel">{group}</div>')
            last_group = group
        href = "index.html" if stem == "README" else f"{stem}.html"
        active = " active" if stem == current_stem else ""
        lines.append(f'    <a class="{("active" if active else "").strip()}" '
                     f'href="{href}">{label}</a>'.replace('class="" ', ""))
    return "\n".join(lines)


def md_links_to_html(html: str) -> str:
    """Rewrite intra-doc links: foo.md -> foo.html, README.md -> index.html."""
    html = re.sub(r'href="README\.md(#[^"]*)?"',
                  lambda m: f'href="index.html{m.group(1) or ""}"', html)
    html = re.sub(r'href="([A-Za-z0-9._-]+)\.md(#[^"]*)?"',
                  lambda m: f'href="{m.group(1)}.html{m.group(2) or ""}"', html)
    return html


def render(stem: str, title: str) -> str:
    src = (DOCS_DIR / f"{stem}.md").read_text(encoding="utf-8")
    md = markdown.Markdown(extensions=MD_EXTENSIONS, extension_configs=MD_EXT_CONFIGS)
    body = md.convert(src)
    body = md_links_to_html(body)

    toc_html = ""
    toc = getattr(md, "toc", "") or ""
    # Only show a TOC when there are at least a couple of headings.
    if toc.count("<li") >= 3:
        toc_html = (f'<div class="toc"><div class="toc-title">On this page</div>'
                    f'{md_links_to_html(toc)}</div>')

    return PAGE.format(stem=stem, title=title, nav=build_nav(stem),
                       toc=toc_html, body=body)


def first_heading(stem: str) -> str:
    for line in (DOCS_DIR / f"{stem}.md").read_text(encoding="utf-8").splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return stem


def main() -> int:
    check = "--check" in sys.argv
    stale: list[str] = []
    built = 0
    for stem, label, _group in NAV:
        md_path = DOCS_DIR / f"{stem}.md"
        if not md_path.exists():
            print(f"  skip   {stem}.md (missing)")
            continue
        title = first_heading(stem)
        html = render(stem, title)
        targets = [DOCS_DIR / f"{stem}.html"]
        if stem == "README":
            targets.append(DOCS_DIR / "index.html")
        for target in targets:
            if check:
                current = target.read_text(encoding="utf-8") if target.exists() else ""
                if current != html:
                    stale.append(target.name)
            else:
                target.write_text(html, encoding="utf-8")
                print(f"  build  {md_path.name:32s} -> {target.name}")
                built += 1
    if check:
        if stale:
            print("stale (run build.py): " + ", ".join(stale))
            return 1
        print("all HTML up to date")
        return 0
    print(f"\ndone — {built} HTML file(s) written.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env python3
"""mdBook preprocessor: build the modding book's chapter tree from its folders, so a new page needs
no edit to SUMMARY.md, and check every page's links.

- A page `X.md` with a folder `X/` beside it gets the folder's pages as its sub-chapters, in file
  name order, each titled by its first heading (and so on, one folder down).
  `<!-- section-pages -->` in such a page becomes a list of links to those sub-chapters.
- A page directly in one of FOLDERS that SUMMARY.md doesn't list joins the end of its part, by title.
- Every relative link, reference destination and `{{#include}}` must reach a file (or a generated
  chapter) and, for a page, an existing heading anchor; root-relative links are refused.
- An absolute URL naming the game's repository or the mod registry names `sunrise-org`, and one naming the
  book names `playsunrise.net/docs/modding`, where it is published; the old Pages addresses are refused.
  Horizon's own projects (the machine sample, the mixins tool) keep their links.
- A page the book doesn't render, one missing a top-level `# ` heading, one titled as a design, a plan or a
  work list, one citing bare `#NNNN` issue numbers in prose, or one with a non-ignored inline `rust` fence
  (instead of `{{#include}}` from `docs/modding/examples`) fails the build.
- The HTML check also refuses duplicate rendered chapter H1 titles across the book, including generated chapters.
"""
import argparse
from collections import Counter
from html.parser import HTMLParser
from itertools import chain
import json
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "docs/modding/src"
LINK = re.compile(r"(\!?\[[^\n]*?\]\()([^\s)]+)(\))")
REFERENCE = re.compile(r"^( {0,3}\[[^\]\n]+\]:[ \t]*<?)([^\s>]+)(>?)", re.MULTILINE)
INCLUDE = re.compile(r"(\{\{#(?:rustdoc_)?include )([^ }]+)(\s*\}\})")
# The game repository and the mod registry are sunrise-org's, and the book is published at
# playsunrise.net/docs/modding, not on a Pages site (`sunrise-org.github.io/sunrise-mods` is the registry's
# index, and keeps its link). A sub-project under another owner (`horizon-machine-sample`, `horizon-mixins`)
# keeps its link too.
STALE_URL = re.compile(
    r"https?://(?:github\.com/(?:arismoko|horizon-rust)/horizon(?:-mods)?(?![\w.-])"
    r"|(?:arismoko|horizon-rust)\.github\.io/horizon-mods(?![\w.-])"
    r"|(?:arismoko|horizon-rust|sunrise-org)\.github\.io/sunrise(?![\w.-]))"
)
# Each page directly in one of these folders is a chapter of the part: the pages SUMMARY.md lists keep
# their place, the rest follow in title order.
FOLDERS = {"Concepts": "concepts", "How-to": "how-to"}
SECTION_PAGES = "<!-- section-pages -->"
# The book documents what the game does. A page titled as one of these is a design, a plan or a work list,
# and lives in docs/design/ instead.
NOT_A_MANUAL = re.compile(r"\b(design|designs|proposal|proposed|draft|roadmap|tracker|status|plan)\b", re.IGNORECASE)
# Reader pages never cite tracker issue or PR numbers (such as `#42` or `#100000`);
# hex colours, Markdown link destinations and explicit heading IDs are excluded before matching.
ISSUE_REF = re.compile(r"(?<![A-Za-z0-9_`/&#-])#(\d+)(?![A-Za-z0-9_])")
# Match fenced code blocks at the top level or inside blockquote containers (`> ```rust`).
FENCE_OPEN = re.compile(r"^((?:[ \t]*>[ \t]?)*)([ \t]*)(`{3,}|~{3,})(.*)$")
INCLUDE_ONLY = re.compile(r"\{\{#(?:rustdoc_)?include ([^\s}]+)\s*\}\}")
COMPILED_EXAMPLE_STEM = re.compile(r"^[a-z0-9_]+$")
HTML_COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)
HEADING_EXPLICIT_ID = re.compile(r"^(#{1,6} .*?)\s*\{#[\w-]+\}\s*$", re.MULTILINE)
BARE_URL = re.compile(r"https?://\S+")


def strip_html_comments_outside_fences(text):
    """Remove HTML comments outside fenced code blocks so commented headings do not count."""
    out = []
    prose_chunk = []
    fenced = ""
    fence_prefix = ""
    for line in text.splitlines(keepends=True):
        if not fenced:
            m = FENCE_OPEN.match(line.rstrip("\r\n"))
            if m:
                if prose_chunk:
                    out.append(HTML_COMMENT.sub("", "".join(prose_chunk)))
                    prose_chunk = []
                fence_prefix, fenced = m.group(1), m.group(3)
                out.append(line)
                continue
            prose_chunk.append(line)
        else:
            out.append(line)
            raw = line.rstrip("\r\n")
            if raw.startswith(fence_prefix):
                rest = raw[len(fence_prefix):]
                close_re = re.compile(rf"^[ \t]*{re.escape(fenced[0])}{{{len(fenced)},}}\s*$")
                if close_re.match(rest):
                    fenced = ""
                    fence_prefix = ""
    if prose_chunk:
        out.append(HTML_COMMENT.sub("", "".join(prose_chunk)))
    return "".join(out)


def h1_headings(text):
    """Yield real, non-empty top-level ATX H1 titles outside fenced blocks and HTML comments."""
    cleaned = strip_html_comments_outside_fences(text)
    fenced = ""
    fence_prefix = ""
    for line in cleaned.splitlines():
        if not fenced:
            m = FENCE_OPEN.match(line)
            if m:
                fence_prefix, fenced = m.group(1), m.group(3)
                continue
            if line.startswith("# "):
                title = line[2:].strip()
                if title:
                    yield title
        else:
            if line.startswith(fence_prefix):
                rest = line[len(fence_prefix):]
                close_re = re.compile(rf"^[ \t]*{re.escape(fenced[0])}{{{len(fenced)},}}\s*$")
                if close_re.match(rest):
                    fenced = ""
                    fence_prefix = ""


def is_compiled_example(target, src):
    """True when `target` is compiled by the `docs/modding/examples` crate (`lib.rs` or `build.rs` discovery)."""
    examples_src = (src.parent / "examples" / "src").resolve()
    resolved = target.resolve()
    if resolved.parent != examples_src or resolved.suffix != ".rs" or not resolved.is_file():
        return False
    return resolved.stem == "lib" or bool(COMPILED_EXAMPLE_STEM.fullmatch(resolved.stem))


def strip_blockquote_prefix(line, prefix):
    if prefix and line.startswith(prefix):
        return line[len(prefix):]
    if prefix:
        # Allow trailing-space differences on empty blockquote lines (`>` vs `> `).
        stripped_prefix = prefix.rstrip(" \t")
        if line.startswith(stripped_prefix):
            return line[len(stripped_prefix):]
    return line


def check_rust_fences(text, path, src):
    """Refuse non-ignored inline Rust fences (including blockquoted ones) and uncompiled include targets."""
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = FENCE_OPEN.match(lines[i])
        if not m:
            i += 1
            continue
        prefix, mark, info = m.group(1), m.group(3), m.group(4).strip()
        close_re = re.compile(rf"^[ \t]*{re.escape(mark[0])}{{{len(mark)},}}\s*$")
        j = i + 1
        body = []
        while j < len(lines):
            inner = strip_blockquote_prefix(lines[j], prefix)
            if close_re.match(inner):
                break
            body.append(inner)
            j += 1
        tokens = [t.strip().lower() for t in re.split(r"[,\s]+", info) if t.strip()]
        if tokens and tokens[0] in ("rust", "rs") and "ignore" not in tokens[1:]:
            non_blank = [line.strip() for line in body if line.strip()]
            matches = [INCLUDE_ONLY.fullmatch(line) for line in non_blank]
            assert non_blank and all(matches), (
                f"{path}:{i + 1}: non-ignored ```{info} block must use {{{{#include}}}} from "
                "docs/modding/examples (or fence fragments as ```rust,ignore)"
            )
            for inc in matches:
                name, _, _ = inc.group(1).partition(":")
                target = (src / path).parent / name
                assert is_compiled_example(target, src), (
                    f"{path}:{i + 1}: Rust include {name} must target a compiled source in "
                    "docs/modding/examples/src/"
                )
        i = j + 1


def check_issue_refs(body, path):
    """Refuse bare issue/PR references (`#42`, `#100000`) in reader prose, excluding link destinations and heading IDs."""
    # Strip reference link definitions, inline link destinations, bare URLs, and explicit heading IDs.
    prose_only = REFERENCE.sub("", body)
    prose_only = LINK.sub(lambda m: m.group(1)[:-1], prose_only)
    prose_only = BARE_URL.sub("", prose_only)
    prose_only = HEADING_EXPLICIT_ID.sub(r"\1", prose_only)
    match = ISSUE_REF.search(prose_only)
    assert not match, f"{path}: bare issue reference {match.group(0)} in reader prose (describe the behaviour instead)"


def headings(text):
    """Yield ATX headings outside fenced examples, using mdBook's heading IDs."""
    fenced = False
    counts = Counter()
    for line in text.splitlines():
        if line.startswith(("```", "~~~")):
            fenced = not fenced
        if not fenced and re.match(r"^#{1,6} ", line):
            title = line.split(" ", 1)[1]
            explicit = re.fullmatch(r"(.*?)\s*\{#([\w-]+)\}\s*", title)
            if explicit:
                yield explicit[1], explicit[2]
                continue
            slug = re.sub(r"[^\w\- ]", "", title.lower()).replace(" ", "-")
            n = counts[slug]
            counts[slug] += 1
            yield title, slug + (f"-{n}" if n else "")


def page_title(text, path):
    return next((title for title, _ in headings(text)), path.stem.replace("-", " ").capitalize())


def prose(text):
    """`text` without fenced code blocks and inline code spans, whose brackets are not links."""
    out, fenced = [], False
    for line in text.splitlines():
        if line.lstrip().startswith(("```", "~~~")):
            fenced = not fenced
            continue
        if not fenced:
            out.append(re.sub(r"(`+)(?:(?!\1).)+?\1", "", line))
    return "\n".join(out)


def check_url(link, where):
    """Refuse an absolute link that sends readers to the game's old repository or the old book site."""
    assert not STALE_URL.match(link), f"{where}: stale URL {link} (use sunrise-org's repository, or playsunrise.net/docs/modding for the book)"


def validate(text, path, src, contents):
    body = prose(text)
    check_issue_refs(body, path)
    check_rust_fences(text, path, src)
    for match in chain(LINK.finditer(body), REFERENCE.finditer(body)):
        url = urlsplit(match[2])
        if url.scheme or url.netloc:
            check_url(match[2], path)
            continue
        assert not unquote(url.path).startswith("/"), f"{path}: root-relative link {match[2]}"
        target = (src / path).parent / unquote(url.path) if url.path else src / path
        target = target.resolve()
        assert target in contents or target.is_file(), f"{path}: missing link target {match[2]}"
        if url.fragment and target.suffix == ".md":
            body = contents[target] if target in contents else target.read_text()
            assert unquote(url.fragment) in {a for _, a in headings(body)}, f"{path}: missing anchor {match[2]}"
    for match in INCLUDE.finditer(text):
        name, _, anchor = match[2].partition(":")
        target = (src / path).parent / name
        assert target.is_file(), f"{path}: missing include {name}"
        if anchor and not anchor[0].isdigit():
            assert f"ANCHOR: {anchor}" in target.read_text(), f"{path}: missing include anchor {anchor}"


def chapter(src, relative, number, parents):
    text = (src / relative).read_text()
    title = page_title(text, src / relative)
    return dict(name=title.replace("`", ""), content=text, path=relative, source_path=relative,
                number=number, parent_names=parents, sub_items=[])


def nest(chapter_, src):
    """Gives a chapter whose page has a folder beside it that folder's pages, in file name order."""
    folder = (src / chapter_["path"]).with_suffix("")
    if not folder.is_dir():
        return
    listed = {item["Chapter"]["path"] for item in chapter_["sub_items"] if "Chapter" in item}
    parents = chapter_["parent_names"] + [chapter_["name"]]
    for path in sorted(folder.glob("*.md")):
        relative = path.relative_to(src).as_posix()
        if relative not in listed:
            chapter_["sub_items"].append({"Chapter": chapter(src, relative, [], parents)})
    for item in chapter_["sub_items"]:
        if "Chapter" in item and item["Chapter"]["path"]:
            nest(item["Chapter"], src)
    here = Path(chapter_["path"]).parent
    links = "\n".join(f"- [{c['name']}]({Path(c['path']).relative_to(here).as_posix()})"
                      for c in (i["Chapter"] for i in chapter_["sub_items"] if "Chapter" in i))
    chapter_["content"] = chapter_["content"].replace(SECTION_PAGES, links)


def chapters(items):
    """Every chapter of the book, depth first."""
    for item in items:
        if "Chapter" in item:
            yield item["Chapter"]
            yield from chapters(item["Chapter"]["sub_items"])


def discover(book, src):
    """Adds each page of FOLDERS that SUMMARY.md doesn't list at the end of its part, by title."""
    items = book["sections"]
    listed = {c["path"] for c in chapters(items) if c["path"]}
    for part, folder in FOLDERS.items():
        start = next((i for i, item in enumerate(items) if item.get("PartTitle") == part), None)
        assert start is not None, f"SUMMARY must have a # {part} part"
        end = next((i for i in range(start + 1, len(items)) if "PartTitle" in items[i]), len(items))
        found = [chapter(src, p.relative_to(src).as_posix(), [], [])
                 for p in (src / folder).glob("*.md") if p.relative_to(src).as_posix() not in listed]
        found.sort(key=lambda c: (c["name"].casefold(), c["path"]))
        items[end:end] = [{"Chapter": c} for c in found]


def renumber(items, prefix):
    """Numbers the chapters again as mdBook does: in order, across parts."""
    index = 0
    for item in items:
        chapter_ = item.get("Chapter")
        if chapter_ is None or chapter_.get("number") is None:
            continue
        index += 1
        chapter_["number"] = prefix + [index]
        renumber(chapter_["sub_items"], chapter_["number"])


def unrendered(book, src):
    """The Markdown sources under `src` that no chapter of `book` renders."""
    rendered = {c["path"] for c in chapters(book["sections"]) if c["path"]}
    return sorted(path.relative_to(src).as_posix() for path in src.rglob("*.md")
                  if path.name != "SUMMARY.md" and path.relative_to(src).as_posix() not in rendered)


def refuse_designs(book):
    """Fails on a chapter whose name or first heading reads as a design or a work list."""
    for chapter_ in chapters(book["sections"]):
        if not chapter_["path"]:
            continue
        titles = [chapter_["name"], *(title for title, _ in headings(chapter_["content"]))][:2]
        for title in titles:
            assert not NOT_A_MANUAL.search(title), (
                f"{chapter_['path']}: \"{title}\" reads as a design or a work list; the book documents what "
                "the game does, and designs, plans and trackers live in docs/design/")


def require_h1(book):
    """Fails on any source chapter missing a top-level `# ` heading."""
    for chapter_ in chapters(book["sections"]):
        if chapter_["path"] and chapter_.get("source_path"):
            assert any(h1_headings(chapter_["content"])), f"{chapter_['path']}: missing top-level `# ` heading"


def expand(book, src):
    discover(book, src)
    for chapter_ in list(chapters(book["sections"])):
        if chapter_["path"] and (src / chapter_["path"]).is_file():
            nest(chapter_, src)
    renumber(book["sections"], [])
    contents = {(src / c["path"]).resolve(): c["content"] for c in chapters(book["sections"]) if c["path"]}
    for chapter_ in chapters(book["sections"]):
        if chapter_["path"] and chapter_.get("source_path"):
            validate(chapter_["content"], chapter_["path"], src, contents)
    missing = unrendered(book, src)
    assert not missing, f"pages the book doesn't render (list them in SUMMARY.md or put them in a folder): {missing}"
    require_h1(book)
    refuse_designs(book)
    return book


VOID_TAGS = frozenset(
    {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}
)
NESTED_HEADING_PARENTS = frozenset(
    {"blockquote", "li", "ul", "ol", "details", "aside", "figure", "table", "td", "th", "pre", "code"}
)


class HtmlLinks(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.ids, self.links = set(), []
        self._in_main = 0
        self._saw_main = False
        self._main_stack = []
        self._outer_nested = []
        self._capture_h1 = None
        self._h1_chunks = []
        self._main_h1 = None
        self._fallback_h1 = None
        self.feed(text)

    @property
    def title(self):
        """The chapter's top-level rendered H1 text, excluding mdBook's menu-title page chrome."""
        return self._main_h1 if self._saw_main else self._fallback_h1

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        if tag == "a" and "href" in attrs:
            self.links.append(attrs["href"])
        if tag == "main":
            self._in_main += 1
            self._saw_main = True
            return
        if self._in_main:
            if tag == "h1" and self._main_stack and self._main_stack[-1] == "p":
                self._main_stack.pop()
            if tag == "h1" and not self._main_stack and self._capture_h1 is None and self._main_h1 is None:
                classes = set((attrs.get("class") or "").split())
                if "menu-title" not in classes:
                    self._capture_h1 = "main"
                    self._h1_chunks = []
            if tag not in VOID_TAGS:
                self._main_stack.append(tag)
        else:
            if tag in NESTED_HEADING_PARENTS:
                self._outer_nested.append(tag)
            elif tag == "h1" and not self._outer_nested and self._capture_h1 is None and self._fallback_h1 is None:
                classes = set((attrs.get("class") or "").split())
                if "menu-title" not in classes:
                    self._capture_h1 = "fallback"
                    self._h1_chunks = []

    def handle_endtag(self, tag):
        if tag == "main" and self._in_main:
            self._in_main -= 1
            if not self._in_main:
                self._main_stack.clear()
            return
        if tag == "h1" and self._capture_h1 is not None:
            text = " ".join("".join(self._h1_chunks).split()) or None
            if self._capture_h1 == "main":
                self._main_h1 = text
            else:
                self._fallback_h1 = text
            self._capture_h1 = None
            self._h1_chunks = []
        if self._in_main:
            for idx in range(len(self._main_stack) - 1, -1, -1):
                if self._main_stack[idx] == tag:
                    del self._main_stack[idx:]
                    break
        elif tag in self._outer_nested:
            for idx in range(len(self._outer_nested) - 1, -1, -1):
                if self._outer_nested[idx] == tag:
                    del self._outer_nested[idx:]
                    break

    def handle_data(self, data):
        if self._capture_h1 is not None:
            self._h1_chunks.append(data)


def check_html(directory, src=SRC):
    pages = sorted(p.relative_to(src).as_posix() for p in src.rglob("*.md") if p.name != "SUMMARY.md")
    seen = set(pages)
    # Discover generated reference chapters that have no checked-in source Markdown file,
    # skipping non-chapter redirect/index HTML files.
    ref_dir = directory / "reference"
    if ref_dir.is_dir():
        for html_file in sorted(ref_dir.glob("*.html")):
            if html_file.name == "index.html":
                continue
            rel = html_file.relative_to(directory).with_suffix(".md").as_posix()
            if rel in seen:
                continue
            raw = html_file.read_text(encoding="utf-8")
            if "http-equiv=\"refresh\"" in raw.lower() or "http-equiv='refresh'" in raw.lower():
                continue
            pages.append(rel)
            seen.add(rel)
    titles, title_errors = {}, []
    for page in pages:
        path = directory / Path(page).with_suffix(".html")
        html = HtmlLinks(path.read_text(encoding="utf-8"))
        for link in html.links:
            url = urlsplit(link)
            if url.scheme or url.netloc:
                check_url(link, page)
                continue
            assert not unquote(url.path).startswith("/"), f"{page}: root-relative link {link}"
            target = (path.parent / unquote(url.path)).resolve() if url.path else path
            assert target.is_relative_to(directory.resolve()) and target.is_file(), (
                f"{page}: missing rendered link {link} (link to repository files with a GitHub URL)")
            if url.fragment:
                assert unquote(url.fragment) in HtmlLinks(target.read_text(encoding="utf-8")).ids, f"{page}: missing rendered anchor {link}"
        if not html.title:
            title_errors.append(f"{page}: missing rendered H1 heading")
        else:
            first = titles.setdefault(html.title.casefold(), page)
            if first != page:
                title_errors.append(f"{page}: duplicate H1 {html.title!r} (also in {first})")
    if title_errors:
        raise AssertionError("\n".join(title_errors))
    print(f"Rendered relative links and anchors: valid in {len(pages)} chapters")


def main():
    if sys.argv[1:2] == ["supports"]:
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--html", type=Path)
    args = parser.parse_args()
    if args.html:
        check_html(args.html.resolve())
    else:
        context, book = json.load(sys.stdin)
        print(json.dumps(expand(book, Path(context["root"]) / context["config"]["book"]["src"])))


if __name__ == "__main__":
    main()

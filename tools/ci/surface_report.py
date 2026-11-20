"""The mod API surface report: what a pull request changes in what mods can name.

Exports the surface of the merge base and of the checked-out tree with the head's `sunrise-stage`, diffs them, and
writes the classified report to the job summary. The base comes from the event: on `pull_request` the checkout is
GitHub's merge commit, whose first parent is the base; on a dispatch with `pr`, GitHub's compare API gives the merge
base of that pull request's head and its base branch. Only that one commit is fetched (`--depth 1`), and only its
`Cargo.toml`, `crates/` and `tools/layering` are unpacked. The engine catalogs are read from source; the workspace
manifest resolves inherited dependencies for crates with injection points. CI compares points and loops, not
listed internals. To audit internals too, use --with-internals; --trees BASE HEAD accepts local source trees.

It passes when the base cannot be found or fetched, saying so in one line. When the head's tool cannot read the
base tree (the base predates a rule or format this tool holds trees to), the base's own `sunrise-stage` is built
(into target/surface-base, which a Dyna slot keeps) and exports the base instead; the report fails, naming both
errors, when that cannot give a surface this tool compares. It fails for a new change note that names an id that did
not change or cannot be read, and, with `gate = true` in tools/ci/surface.toml (on), for a breaking change no new note
names.
"""

from argparse import ArgumentParser
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
import json
import os
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
NOTES = "crates/sunrise-mod-vocab/changes"
# GitHub refuses a step summary over 1 MiB; the full report stays in the step log.
# Counted in bytes of UTF-8, frame included.
SUMMARY_LIMIT = 200_000


def gate(config: Path) -> bool:
    value = tomllib.loads(config.read_text())["surface"]["gate"]
    if not isinstance(value, bool):
        raise SystemExit(f"{config}: surface.gate must be true or false")
    return value


def merge_parent(commit_text: str) -> str | None:
    """The first parent of a merge commit (`git cat-file -p` text), or None for any other commit."""
    parents = [line.split()[1] for line in commit_text.splitlines() if line.startswith("parent ")]
    return parents[0] if len(parents) == 2 else None


def base_commit(event: str, pr: str, git, api) -> tuple[str | None, str]:
    """The commit to compare against, or None with the reason."""
    if event == "pull_request":
        parent = merge_parent(git("cat-file", "-p", "HEAD"))
        return (parent, "") if parent else (None, "the checkout is not a pull request's merge commit")
    if event == "workflow_dispatch" and pr:
        # The checked-out revision, not the pull request's head now: a push after the run
        # picked its revision would give a base from another history.
        head = git("rev-parse", "HEAD").strip()
        pull = api(f"pulls/{pr}")
        compare = api(f"compare/{pull['base']['ref']}...{head}")
        return compare["merge_base_commit"]["sha"], ""
    return None, f"no base for a {event or 'local'} run"


def github_api(path: str) -> dict:
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/{path}",
        headers={"Authorization": f"Bearer {os.environ['GH_TOKEN']}", "Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(request, timeout=20) as answer:
        return json.load(answer)


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def unpack(sha: str, into: Path, paths=("Cargo.toml", "crates", "tools/layering")) -> None:
    git("fetch", "-q", "--no-tags", "--depth", "1", "origin", sha)
    archive = subprocess.Popen(["git", "archive", sha, *paths], cwd=ROOT, stdout=subprocess.PIPE)
    subprocess.run(["tar", "-x", "-C", str(into)], stdin=archive.stdout, check=True)
    if archive.wait() != 0:
        raise subprocess.CalledProcessError(archive.returncode, "git archive")


def base_tool(sha: str, work: Path) -> str:
    """The base commit's own sunrise-stage: its whole tree, built as verify builds the head's (test profile)."""
    tree = work / "base-tool"
    tree.mkdir()
    unpack(sha, tree, ())
    target = ROOT / "target" / "surface-base"
    built = subprocess.run(["cargo", "build", "--locked", "--profile", "test", "-p", "sunrise-mod-api-gen",
                            "--bin", "sunrise-stage", "--target-dir", str(target)],
                           cwd=tree, capture_output=True, text=True)
    if built.returncode != 0:
        raise RuntimeError(f"building the base's sunrise-stage failed: {last_line(built.stderr)}")
    return str(target / "debug" / "sunrise-stage")


def last_line(text: str) -> str:
    lines = [line for line in text.splitlines() if line.strip() and not line.startswith("unclassified:")]
    return lines[-1] if lines else "no message"


def report(tool: str, base: Path, head: Path, work: Path, gate_on: bool, *,
           with_internals: bool = False, own_base_tool: Callable[[], str] | None = None) -> tuple[int, str]:
    """Exports both trees at once, diffs them; (exit code, report text). `own_base_tool` gives the base's own
    sunrise-stage, for a base the head's cannot read."""
    # The base's records may predate the call names: its export names none (`--without-calls`),
    # which no comparison reads.
    catalogs = "--with-internals" if with_internals else "--with-points-and-loops"

    def export(name: str, exporter: str, tree: Path, flags: list[str]) -> subprocess.CompletedProcess:
        return subprocess.run([exporter, "surface", str(tree), str(work / f"{name}.json"), *flags, catalogs],
                              capture_output=True, text=True)

    with ThreadPoolExecutor(max_workers=2) as pool:
        # Drain both subprocesses' diagnostics while they run so a full pipe cannot stall an export.
        exports = {name: pool.submit(export, name, tool, tree, flags)
                   for name, tree, flags in (("base", base, ["--without-calls"]), ("head", head, []))}
    # This tree failing is a broken generator, which no other step would catch.
    head_export, base_export = exports["head"].result(), exports["base"].result()
    if head_export.returncode != 0:
        return 1, f"The surface export of this tree failed: {last_line(head_export.stderr)}.\n"
    note = ""
    if base_export.returncode != 0:
        # An older tree this tool holds to a newer rule or format: the base's own tool reads it.
        why = f"the head's sunrise-stage cannot read the base tree ({last_line(base_export.stderr)})"
        if own_base_tool is None:
            return 1, f"No surface report: {why}.\n"
        try:
            own = export("base", own_base_tool(), base, ["--without-calls"])
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            return 1, f"No surface report: {why}, and the base's own could not be used: {error}.\n"
        if own.returncode != 0:
            return 1, f"No surface report: {why}, nor can the base's own ({last_line(own.stderr)}).\n"
        note = f"The base was exported by its own sunrise-stage: {why}.\n"
    code, text = compare(tool, base, head, work, gate_on, with_internals=with_internals)
    return code, note + text


def compare(tool: str, base: Path, head: Path, work: Path, gate_on: bool, *,
            with_internals: bool) -> tuple[int, str]:
    """Run the gate on the exported documents and the trees' change notes."""
    command = [tool, "surface-diff", str(work / "base.json"), str(work / "head.json"),
               "--notes", str(head / NOTES), "--base-notes", str(base / NOTES), "--json", str(work / "diff.json")]
    if gate_on:
        command.append("--gate")
    diff = subprocess.run(command, capture_output=True, text=True)
    # A base surface of another schema (one the base's own tool wrote) cannot be compared: say so, and fail.
    base_schema = f"{work / 'base.json'}: surface schema "
    if diff.returncode == 2 and base_schema in diff.stderr:
        return 1, f"No surface report: the head's sunrise-stage cannot read the base surface ({last_line(diff.stderr)}).\n"
    scope = "" if with_internals else (
        "CI does not compare listed internals; use surface_report.py --with-internals for a local audit.\n")
    return (0 if diff.returncode == 0 else 1), scope + diff.stdout + diff.stderr


def summary(text: str) -> str:
    """The report for the job summary, at most SUMMARY_LIMIT bytes of UTF-8 in all."""
    head, foot = "### Mod API surface\n\n```text\n", "```\n"
    # Room for the frame and for the line saying what was cut.
    budget = SUMMARY_LIMIT - len((head + foot).encode()) - 64
    data = text.encode()
    if len(data) > budget:
        kept = data[:budget]
        line_end = kept.rfind(b"\n")
        # A line longer than the budget is cut inside itself, on a character boundary.
        kept = kept[: line_end + 1] if line_end >= 0 else kept
        shown = kept.decode(errors="ignore")
        rest = text[len(shown) :].count("\n")
        lines = "line" if rest == 1 else "lines"
        text = shown.rstrip("\n") + f"\n… cut here; {rest} more {lines} in the step log\n"
    return head + text + foot


def main(argv=None) -> int:
    parser = ArgumentParser(description="Compare mod API surfaces and published engine catalogs.")
    parser.add_argument("--with-internals", action="store_true", help="include advisory listed-internal changes")
    parser.add_argument("--trees", nargs=2, metavar=("BASE", "HEAD"), type=Path,
                        help="compare two local source trees instead of fetching the CI base")
    args = parser.parse_args(argv)
    started = time.monotonic()
    tool = os.environ["SUNRISE_STAGE_TOOL"]
    gate_on = gate(ROOT / "tools/ci/surface.toml")
    with tempfile.TemporaryDirectory(dir=os.environ.get("RUNNER_TEMP")) as directory:
        work = Path(directory)
        if args.trees:
            base, head = args.trees
            code, text = report(tool, base, head, work, gate_on, with_internals=args.with_internals)
        else:
            base = work / "base"
            try:
                sha, why = base_commit(os.environ.get("EVENT", ""), os.environ.get("PR", ""), git, github_api)
                if sha:
                    base.mkdir()
                    unpack(sha, base)
            except (OSError, subprocess.CalledProcessError, KeyError, ValueError) as error:
                sha, why = None, f"the base could not be fetched: {error}"
            if sha:
                fetched = time.monotonic() - started
                code, text = report(tool, base, ROOT, work, gate_on, with_internals=args.with_internals,
                                    own_base_tool=lambda: base_tool(sha, work))
                text += f"Base {sha}; fetch {fetched:.1f}s, total {time.monotonic() - started:.1f}s\n"
            else:
                code, text = 0, f"No surface report: {why}.\n"
    print(text, end="")
    if summary_path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary_path, "a") as out:
            out.write(summary(text))
    return code


if __name__ == "__main__":
    sys.exit(main())

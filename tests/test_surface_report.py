"""The surface report step: which base it compares, when it passes, when it fails."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import urllib.error
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("surface_report", ROOT / "tools/ci/surface_report.py")
surface_report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(surface_report)

MERGE = "tree t\nparent aaaa\nparent bbbb\nauthor x\n\nMerge bbbb into aaaa\n"
SINGLE = "tree t\nparent aaaa\nauthor x\n\nA commit\n"


class BaseCommitTests(unittest.TestCase):
    def test_a_pull_request_compares_against_its_merge_commits_first_parent(self):
        sha, _ = surface_report.base_commit("pull_request", "", lambda *a: MERGE, None)
        self.assertEqual(sha, "aaaa")
        sha, why = surface_report.base_commit("pull_request", "", lambda *a: SINGLE, None)
        self.assertIsNone(sha)
        self.assertIn("not a pull request's merge commit", why)

    def test_a_dispatch_asks_for_the_merge_base_of_the_checked_out_revision(self):
        asked = []

        def api(path):
            asked.append(path)
            if path == "pulls/12":
                # The pull request has moved on since the run picked its revision.
                return {"base": {"ref": "main"}, "head": {"sha": "eeee"}}
            return {"merge_base_commit": {"sha": "dddd"}}

        git = lambda *a: "cccc\n" if a == ("rev-parse", "HEAD") else ""
        sha, _ = surface_report.base_commit("workflow_dispatch", "12", git, api)
        self.assertEqual((sha, asked), ("dddd", ["pulls/12", "compare/main...cccc"]))

    def test_a_push_has_no_base(self):
        sha, why = surface_report.base_commit("push", "", None, None)
        self.assertIsNone(sha)
        self.assertEqual(why, "no base for a push run")


class GateTests(unittest.TestCase):
    def test_the_tracked_switch_is_read_and_must_be_a_boolean(self):
        self.assertIs(surface_report.gate(ROOT / "tools/ci/surface.toml"), True)
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory, "surface.toml")
            config.write_text("[surface]\ngate = true\n")
            self.assertIs(surface_report.gate(config), True)
            config.write_text("[surface]\ngate = \"yes\"\n")
            with self.assertRaises(SystemExit):
                surface_report.gate(config)


@unittest.skipUnless(shutil.which("bash"), "needs bash for the stub tool")
class ReportTests(unittest.TestCase):
    """report() against a stub sunrise-stage whose behaviour each test sets."""

    def run_report(self, base_export=0, diff_exit=0, gate_on=False, head_export=0, diff_err="",
                   with_internals=False, own_export=None):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            tool = work / "sunrise-stage"
            tool.write_text(f"""#!/bin/bash
echo "$@" >> {work}/calls
if [ "$1" = surface ]; then
  case "$2" in *base) [ {base_export} = 0 ] || {{ echo 'unclassified: x' >&2; echo 'sunrise-stage surface: unknown key `access`' >&2; exit 1; }};; esac
  case "$2" in *head) [ {head_export} = 0 ] || {{ echo 'sunrise-stage surface: a.rs: parse error' >&2; exit 1; }};; esac
  echo '{{}}' > "$3"; exit 0
fi
echo "Surface: report"; [ -z "{diff_err}" ] && [ {diff_exit} != 0 ] && echo "breaking change without a change note: fn:a" >&2
[ -n "{diff_err}" ] && echo "sunrise-stage: $2: {diff_err}" >&2
exit {diff_exit}
""")
            tool.chmod(0o755)
            # The base's own tool, when a test gives one: it exports with `own_export` as its exit status.
            own = None
            if own_export is not None:
                own_tool = work / "own-stage"
                own_tool.write_text(f"""#!/bin/bash
echo "own $@" >> {work}/calls
[ {own_export} = 0 ] || {{ echo 'sunrise-stage surface: base.rs: older form' >&2; exit 1; }}
echo '{{}}' > "$3"
""")
                own_tool.chmod(0o755)
                own = lambda: str(own_tool)  # noqa: E731
            (work / "base").mkdir()
            code, text = surface_report.report(str(tool), work / "base", work / "head", work, gate_on,
                                               with_internals=with_internals, own_base_tool=own)
            return code, text, (work / "calls").read_text()

    def test_a_base_the_head_tool_cannot_read_is_exported_by_the_bases_own_tool(self):
        code, text, calls = self.run_report(base_export=1, own_export=0)
        self.assertEqual(code, 0)
        self.assertTrue(text.startswith("The base was exported by its own sunrise-stage: the head's sunrise-stage "
                                        "cannot read the base tree (sunrise-stage surface: unknown key `access`).\n"),
                        text)
        self.assertIn("Surface: report\n", text)
        own = [call for call in calls.splitlines() if call.startswith("own ")]
        self.assertEqual(len(own), 1)
        self.assertIn("--without-calls", own[0])
        self.assertIn("surface-diff", calls)

    def test_a_base_neither_tool_can_read_fails_naming_both(self):
        code, text, calls = self.run_report(base_export=1, own_export=1)
        self.assertEqual(code, 1)
        self.assertEqual(text, "No surface report: the head's sunrise-stage cannot read the base tree "
                               "(sunrise-stage surface: unknown key `access`), nor can the base's own "
                               "(sunrise-stage surface: base.rs: older form).\n")
        self.assertNotIn("surface-diff", calls)
        # With no base tool to fall back on (a local comparison of two trees), the unreadable base fails too.
        code, text, _ = self.run_report(base_export=1)
        self.assertEqual(code, 1)
        self.assertTrue(text.startswith("No surface report: the head's sunrise-stage cannot read the base tree"), text)

    def test_a_base_surface_of_another_schema_fails_with_one_line(self):
        code, text, _ = self.run_report(diff_exit=2, diff_err="surface schema 1, this tool reads 2")
        self.assertEqual(code, 1)
        self.assertTrue(text.startswith("No surface report: the head's sunrise-stage cannot read the base surface "), text)
        self.assertIn("surface schema 1, this tool reads 2", text)

    def test_this_tree_failing_to_export_fails_the_step(self):
        for base_export in (0, 1):
            code, text, calls = self.run_report(head_export=1, base_export=base_export)
            self.assertEqual(code, 1)
            self.assertEqual(text, "The surface export of this tree failed: sunrise-stage surface: a.rs: parse error.\n")
            self.assertNotIn("surface-diff", calls)

    def test_the_diff_decides_and_the_gate_is_passed_on(self):
        code, text, calls = self.run_report()
        self.assertEqual(code, 0)
        self.assertIn("CI does not compare listed internals", text)
        self.assertIn("--with-internals", text)
        self.assertIn("Surface: report\n", text)
        self.assertNotIn("--gate", calls)
        self.assertNotIn("--with-internals", calls)
        exports = [call.split() for call in calls.splitlines() if call.startswith("surface ")]
        self.assertEqual(len(exports), 2)
        for call in exports:
            self.assertIn("--with-points-and-loops", call)
        self.assertIn("--notes ", calls)
        code, text, calls = self.run_report(diff_exit=1, gate_on=True)
        self.assertEqual(code, 1)
        self.assertIn("fn:a", text)
        self.assertIn("--gate", calls)
        # An unreadable note (exit 2) fails too.
        code, _, _ = self.run_report(diff_exit=2)
        self.assertEqual(code, 1)

    def test_the_audit_exports_internals_on_both_sides(self):
        code, text, calls = self.run_report(with_internals=True)
        self.assertEqual((code, text), (0, "Surface: report\n"))
        exports = [call for call in calls.splitlines() if call.startswith("surface ")]
        self.assertEqual(len(exports), 2)
        for call in exports:
            self.assertIn("--with-internals", call.split())


@unittest.skipUnless(os.environ.get("SUNRISE_STAGE_TOOL"), "set SUNRISE_STAGE_TOOL to test the real gate")
class EngineCatalogGateTests(unittest.TestCase):
    ITEMS = [
        {"id": "point:sunrise_client::game::Game::tick", "kind": "point", "hash": "point-1",
         "name": "tick", "signature_hash": "old-point"},
        {"id": "loop:entities.tick", "kind": "loop", "hash": "loop-1", "entry": "Game::tick_entities",
         "generic": "tick_entities_with", "slot": "TickSlot", "side": "server"},
        {"id": "internal:fn:sunrise_client::game::Game::tick [feature = \"fixture\"]",
         "kind": "internal", "hash": "internal-1", "signature": "fn tick(&self)",
         "signature_hash": "old-internal"},
    ]

    def compare(self, old, new, *, notes=()):
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            for name, records in (("base", old), ("head", new)):
                (work / f"{name}.json").write_text(json.dumps({"schema": 2, "hash": name, "engine_items": records}))
            if notes:
                changes = work / "head" / surface_report.NOTES
                changes.mkdir(parents=True)
                (changes / "engine-change.toml").write_text(f'ids = {json.dumps(notes)}\nwhy = "Use the new sites."\n')
            return surface_report.compare(os.environ["SUNRISE_STAGE_TOOL"], work / "base", work / "head", work, True,
                                          with_internals=True)

    def test_undeclared_point_and_loop_removals_fail_and_declared_removals_pass(self):
        code, text = self.compare(self.ITEMS, [])
        self.assertEqual(code, 1, text)
        ids = [item["id"] for item in self.ITEMS[:2]]
        for record in ids:
            self.assertIn(f"- {record} (removed)", text)
            self.assertIn(f"breaking change without a change note: {record}", text)
        code, text = self.compare(self.ITEMS, [], notes=ids)
        self.assertEqual(code, 0, text)
        self.assertNotIn("breaking change without a change note:", text)

    def test_undeclared_internal_removal_is_advisory_and_listed(self):
        internal = self.ITEMS[2]
        code, text = self.compare([internal], [])
        self.assertEqual(code, 0, text)
        self.assertIn("Surface: advisory (0 breaking, 0 conditional, 0 additive, 1 advisory;", text)
        self.assertIn("Listed internals (advisory): 0 added, 1 removed, 0 changed", text)
        self.assertIn(f"- {internal['id']} (removed)", text)
        self.assertNotIn("breaking change without a change note:", text)

    def test_additions_pass_and_each_kind_has_a_count(self):
        code, text = self.compare([], self.ITEMS)
        self.assertEqual(code, 0, text)
        self.assertIn("0 breaking, 0 conditional, 2 additive, 1 advisory", text)
        for kind in ("Injection points", "Published loops", "Listed internals (advisory)"):
            self.assertIn(f"{kind}: 1 added, 0 removed, 0 changed", text)

    def test_changed_contracts_keep_their_ids(self):
        point, loop, internal = self.ITEMS
        code, text = self.compare(self.ITEMS, [
            dict(point, hash="point-2", signature_hash="new-point"),
            dict(loop, hash="loop-2", slot="NewTickSlot"),
            dict(internal, hash="internal-2", signature="fn tick(&mut self)", signature_hash="new-internal"),
        ])
        self.assertEqual(code, 1, text)
        self.assertIn("2 breaking, 0 conditional, 0 additive, 1 advisory", text)
        for item, detail in zip(self.ITEMS, ("`old-point` → `new-point`", "`TickSlot` → `NewTickSlot`",
                                            "`fn tick(&self)` → `fn tick(&mut self)`")):
            self.assertIn(f"- {item['id']} (changed)", text)
            self.assertIn(detail, text)
        self.assertNotIn("(removed)", text)
        self.assertNotIn("(added)", text)


class SummaryTests(unittest.TestCase):
    def size(self, out: str) -> int:
        return len(out.encode())

    def test_a_long_report_is_cut_at_a_line_end(self):
        text = "".join(f"- fn:item{i} (added)\n" for i in range(20_000))
        out = surface_report.summary(text)
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertRegex(out, r"\(added\)\n… cut here; \d+ more lines in the step log\n```\n$")
        self.assertEqual(surface_report.summary("short\n"), "### Mod API surface\n\n```text\nshort\n```\n")

    def test_a_cut_inside_a_multi_byte_character_keeps_whole_characters(self):
        # "é" is two bytes, so the byte budget is half the characters and lands inside one.
        for pad in ("", "x"):
            out = surface_report.summary(pad + "é" * 200_000 + "\nnext\n")
            self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
            self.assertGreater(self.size(out), surface_report.SUMMARY_LIMIT - 200)
            self.assertNotIn("\ufffd", out)
            self.assertIn("é\n… cut here; 2 more lines in the step log\n", out)

    def test_one_line_longer_than_the_cap_is_cut(self):
        out = surface_report.summary("a" * 300_000 + "\n")
        self.assertLessEqual(self.size(out), surface_report.SUMMARY_LIMIT)
        self.assertGreater(out.count("a"), 199_000)
        self.assertTrue(out.endswith("a\n… cut here; 1 more line in the step log\n```\n"))


class MainTests(unittest.TestCase):
    """main(): the base it fetches, what it prints, what it writes to the job summary."""

    def run_main(self, event="pull_request", pr="", argv=(), **patches):
        with tempfile.TemporaryDirectory() as directory:
            summary_file = Path(directory) / "summary.md"
            env = {"SUNRISE_STAGE_TOOL": "/no/tool", "EVENT": event, "PR": pr,
                   "GITHUB_STEP_SUMMARY": str(summary_file), "RUNNER_TEMP": directory}
            with mock.patch.dict(os.environ, env), contextlib.redirect_stdout(io.StringIO()) as out, \
                    contextlib.ExitStack() as stack:
                for name, value in patches.items():
                    stack.enter_context(mock.patch.object(surface_report, name, value))
                code = surface_report.main(argv)
            return code, out.getvalue(), summary_file.read_text()

    def test_an_unfetchable_base_reports_and_passes(self):
        def git(*args):
            raise subprocess.CalledProcessError(128, ["git", *args])

        code, out, summary = self.run_main(git=git)
        self.assertEqual(code, 0)
        self.assertTrue(out.startswith("No surface report: the base could not be fetched: "), out)
        self.assertIn("No surface report", summary)

    def test_a_dispatch_whose_api_call_fails_reports_and_passes(self):
        def api(path):
            raise urllib.error.URLError("no route")

        code, out, _ = self.run_main("workflow_dispatch", "7", github_api=api, git=lambda *a: "cccc\n")
        self.assertEqual(code, 0)
        self.assertIn("the base could not be fetched", out)

    def test_a_report_is_printed_written_and_its_code_returned(self):
        fetched = []
        code, out, summary = self.run_main(
            base_commit=lambda *a: ("abcd", ""),
            unpack=lambda sha, into: fetched.append(sha),
            report=lambda *a, **kw: (1, "Surface: breaking\n"))
        self.assertEqual((code, fetched), (1, ["abcd"]))
        self.assertRegex(out, r"^Surface: breaking\nBase abcd; fetch \d+\.\ds, total \d+\.\ds\n$")
        self.assertTrue(summary.startswith("### Mod API surface\n\n```text\nSurface: breaking\nBase abcd"))

    def test_local_audit_uses_the_supplied_trees_without_fetching(self):
        with mock.patch.object(surface_report, "report", return_value=(0, "Surface: advisory\n")) as report, \
             mock.patch.object(surface_report, "base_commit") as find_base:
            code, out, _ = self.run_main(argv=["--with-internals", "--trees", "before", "after"])
        self.assertEqual((code, out), (0, "Surface: advisory\n"))
        self.assertEqual(report.call_args.args[1:3], (Path("before"), Path("after")))
        self.assertTrue(report.call_args.kwargs["with_internals"])
        find_base.assert_not_called()


if __name__ == "__main__":
    unittest.main()

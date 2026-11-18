import contextlib
import io
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools import remove_launcher_tree


class RemoveLauncherTreeTests(unittest.TestCase):
    def test_rewrites_once_leaves_symlinks_alone_and_a_second_run_changes_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "repo"
            outside = Path(directory) / "outside.md"
            outside.write_text("see launcher/src/a.rs\n")
            (root / "launcher/src").mkdir(parents=True)
            (root / "launcher/src/a.rs").write_text("fn main() {}\n")
            (root / "docs").mkdir()
            (root / "docs/x.md").write_text("see launcher/src/a.rs\n")
            (root / "docs/link.md").symlink_to(outside)
            git = ["git", "-c", "user.name=t", "-c", "user.email=t@example.com"]
            env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull}
            for command in (["init", "-q"], ["add", "-A"], ["commit", "-q", "-m", "x"]):
                subprocess.run(git + command, cwd=root, check=True, env=env)
            with patch.object(remove_launcher_tree, "ROOT", root), patch.dict(os.environ, env), \
                    patch("sys.argv", ["remove_launcher_tree.py"]), contextlib.redirect_stdout(io.StringIO()):
                remove_launcher_tree.main()
                self.assertFalse((root / "launcher").exists())
                rewritten = (root / "docs/x.md").read_text()
                self.assertEqual(rewritten, f"see {remove_launcher_tree.REPO}/blob/main/src/a.rs\n")
                self.assertEqual(outside.read_text(), "see launcher/src/a.rs\n")
                remove_launcher_tree.main()
            self.assertEqual((root / "docs/x.md").read_text(), rewritten)


if __name__ == "__main__":
    unittest.main()

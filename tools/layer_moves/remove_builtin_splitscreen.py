#!/usr/bin/env python3
"""Remove the built-in splitscreen modules, smoke driver and locale entries.

Run from the repository root, on the tree before the splitscreen cutover.
Controller connection notices survive, including the reviewed translations.
"""

import json
import re
from pathlib import Path


REMOVED = (
    "crates/sunrise-client/src/game/runtime/split",
    "crates/sunrise-client-render/src/split_layout.rs",
    "crates/sunrise-client-render/src/split_map.rs",
    "crates/sunrise-client-render/src/split_profiles.rs",
    "crates/sunrise-client-render/src/split_profiles",
    "crates/sunrise-client-render/src/split_ui.rs",
    "tools/splitscreen_e2e.py",
)
NOTICE_PREFIX = "sunrise.splitscreen.prompt."
NOTICE_KEYS = {
    f"{NOTICE_PREFIX}{suffix}"
    for suffix in (
        "controller_connected",
        "controller_disconnected",
        "controller_disconnected.hint",
    )
}
SPLIT_KEY = re.compile(r'"sunrise\.(?:split(?:screen)?\.|extras\.splitscreen[." ]|pages\.(?:open\.)?split_)')


def update_locale(path):
    source = path.read_text(encoding="utf-8")
    lines = []
    for line in source.splitlines(keepends=True):
        key = line.strip().partition('":')[0].removeprefix('"')
        if key in NOTICE_KEYS:
            line = line.replace(NOTICE_PREFIX, "sunrise.controller.notice.", 1)
        elif SPLIT_KEY.search(line.partition(":")[0] + '"'):
            continue
        lines.append(line)
    updated = re.sub(r",(\n\s*})\s*\Z", r"\1\n", "".join(lines))
    json.loads(updated)
    if updated != source:
        path.write_text(updated, encoding="utf-8")


def main():
    for root in ("crates/sunrise-client/assets/sunrise/lang", "tools/lang_review/game"):
        for path in sorted(Path(root).glob("*.json")):
            update_locale(path)
    for name in REMOVED:
        path = Path(name)
        if path.is_dir():
            for child in sorted(path.glob("*.rs")):
                child.unlink()
            path.rmdir()
        else:
            path.unlink()


if __name__ == "__main__":
    main()

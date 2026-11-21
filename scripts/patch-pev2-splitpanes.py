#!/usr/bin/env python3
"""Apply pinned PEV2 1.25.0 Splitpanes resize safety fixes.

Run this after updating the vendored PEV2 distribution. This patch is deliberately
version-specific and refuses to modify an unrecognized minified bundle.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parent.parent
BUNDLE = ROOT / "ui/vendor/pev2/pev2.umd.js"
VERSION_FILE = ROOT / "ui/vendor/pev2/VERSIONS.txt"

# Splitpanes schedules mousemove processing for the next animation frame.
# By the time the callback fires, the pane may have been unmounted.
OLD_FRAME = (
    "v=e=>{f.value.mouseDown&&(e.preventDefault(),f.value.dragging||"
    "(window.getSelection()?.removeAllRanges(),f.value.dragging=!0),"
    "requestAnimationFrame(()=>{E(w(e)),le(`resize`,{event:e},!0)}))}"
)
NEW_FRAME = (
    "v=e=>{f.value.mouseDown&&u.value&&(e.preventDefault(),f.value.dragging||"
    "(window.getSelection()?.removeAllRanges(),f.value.dragging=!0),"
    "requestAnimationFrame(()=>{f.value.mouseDown&&u.value&&s.value.length>1&&"
    "(E(w(e)),le(`resize`,{event:e},!0))}))}"
)

# A document-level mousemove handler must not survive a destroyed pane.
OLD_UNMOUNT = "(0,t.onBeforeUnmount)(()=>d.value=!1);let ue=()"
NEW_UNMOUNT = (
    "(0,t.onBeforeUnmount)(()=>{d.value=!1,f.value.mouseDown=!1,"
    "f.value.activeSplitter=null,g(),document.documentElement.style.cursor=``});let ue=()"
)


# PEV2 Plan page assumes that Splitpanes always emits an array with a first
# pane. During resize/teardown an empty payload can arrive: e[0].size throws.
OLD_RESIZE = "onResize:c[4]||=e=>_.diagramWidth=e[0].size"
NEW_RESIZE = (
    "onResize:c[4]||=e=>{Array.isArray(e)&&e[0]&&"
    "Number.isFinite(e[0].size)&&(_.diagramWidth=e[0].size)}"
)


def patch_content(source: str) -> tuple[str, bool]:
    result = source
    changed = False
    for original, patched in (
        (OLD_FRAME, NEW_FRAME),
        (OLD_UNMOUNT, NEW_UNMOUNT),
        (OLD_RESIZE, NEW_RESIZE),
    ):
        if result.count(patched) == 1 and result.count(original) == 0:
            continue  # Already applied.
        if result.count(original) != 1 or result.count(patched) != 0:
            raise ValueError(
                "PEV2 bundle does not match the expected 1.25.0 Splitpanes/Plan resize code; "
                "refusing an unsafe vendor edit."
            )
        result = result.replace(original, patched, 1)
        changed = True
    return result, changed


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Verify the patch without changing the bundle")
    parser.add_argument("--bundle", type=Path, default=BUNDLE)
    parser.add_argument("--versions", type=Path, default=VERSION_FILE)
    args = parser.parse_args()

    if "pev2=1.25.0" not in args.versions.read_text(encoding="utf-8").splitlines():
        parser.error("Only the vendored PEV2 1.25.0 is supported")

    before = args.bundle.read_text(encoding="utf-8")
    after, changed = patch_content(before)
    if args.check:
        print("PEV2 1.25.0 Splitpanes: verified (patched)" if not changed else "PEV2 1.25.0 Splitpanes: patch applicable")
        return
    if changed:
        mode = args.bundle.stat().st_mode
        fd, temp_name = tempfile.mkstemp(prefix=".pev2-splitpanes-", dir=args.bundle.parent)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as output:
                output.write(after)
            os.chmod(temp_name, mode)
            os.replace(temp_name, args.bundle)
        finally:
            if os.path.exists(temp_name):
                os.unlink(temp_name)
    print("PEV2 1.25.0 Splitpanes: patched" if changed else "PEV2 1.25.0 Splitpanes: already patched")


if __name__ == "__main__":
    main()

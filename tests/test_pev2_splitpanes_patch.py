"""Regression checks for the pinned PEV2 1.25.0 resize fix."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/patch-pev2-splitpanes.py"
SPEC = importlib.util.spec_from_file_location("pev2_splitpanes_patch", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SplitpanesPatchTests(unittest.TestCase):
    def test_guarded_frame_and_unmount_cleanup(self):
        source = MODULE.OLD_FRAME + ";" + MODULE.OLD_UNMOUNT + ";" + MODULE.OLD_RESIZE
        patched, changed = MODULE.patch_content(source)
        self.assertTrue(changed)
        self.assertIn("f.value.mouseDown&&u.value&&s.value.length>1", patched)
        self.assertIn("g(),document.documentElement.style.cursor", patched)
        self.assertIn("Number.isFinite(e[0].size)", patched)

        again, changed_again = MODULE.patch_content(patched)
        self.assertFalse(changed_again)
        self.assertEqual(patched, again)

    def test_empty_resize_payload_keeps_previous_width(self):
        # The guard must not access e[0].size until a first pane exists.
        # Preserve the last valid diagramWidth for missing/invalid payloads.
        handler = MODULE.NEW_RESIZE.split("||=", 1)[1]
        import shutil
        if not shutil.which("node"):
            self.skipTest("Node.js is not installed")
        js = (
            "let _ = {diagramWidth: 20}; "
            "const fn = " + handler + "; "
            "for (const value of [[], null, undefined, [{}], [{size: NaN}]]) { "
            "fn(value); if (_.diagramWidth !== 20) process.exit(1); } "
            "fn([{size: 35}]); if (_.diagramWidth !== 35) process.exit(2);"
        )
        result = subprocess.run(["node", "-e", js], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_refuses_unknown_vendor_layout(self):
        with self.assertRaisesRegex(ValueError, "does not match"):
            MODULE.patch_content("other PEV2 version")

    def test_current_vendor_is_patched(self):
        content = (ROOT / "ui/vendor/pev2/pev2.umd.js").read_text(encoding="utf-8")
        patched, changed = MODULE.patch_content(content)
        self.assertFalse(changed, "Run scripts/patch-pev2-splitpanes.py")
        self.assertEqual(content, patched)

    def test_rejects_wrong_version_before_writing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "pev2.umd.js"
            bundle.write_text(MODULE.OLD_FRAME + ";" + MODULE.OLD_UNMOUNT + ";" + MODULE.OLD_RESIZE)
            versions = root / "VERSIONS.txt"
            versions.write_text("pev2=1.26.0\n")
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--bundle", str(bundle), "--versions", str(versions)],
                text=True,
                capture_output=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Only the vendored PEV2 1.25.0", result.stderr)
            self.assertIn(MODULE.OLD_FRAME, bundle.read_text())


if __name__ == "__main__":
    unittest.main()

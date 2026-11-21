"""Regression checks for the optional localized Rust pair classification."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "i18n-audit.py"
spec = importlib.util.spec_from_file_location("pgxray_i18n_audit", SCRIPT)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AuditLocalizedPairsTests(unittest.TestCase):
    def test_rust_pair_is_not_classified_as_untranslated(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            core = Path(directory) / "crates" / "core" / "src"
            core.mkdir(parents=True)
            (core / "lib.rs").write_text(
                'let translated = crate::i18n::text("Готово", "Ready");\n'
                'let untranslated = "Нужно перевести";\n', encoding="utf-8"
            )
            rows, _ = module.scan(Path(directory), classify_localized_pairs=True)
            self.assertEqual(
                {row["text"]: row["classification"] for row in rows},
                {"Готово": "localized-pair", "Нужно перевести": "backend-review"},
            )
            rows_legacy, _ = module.scan(Path(directory))
            self.assertTrue(all(row["classification"] == "backend-review" for row in rows_legacy))

    def test_multiline_pair(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            core = Path(directory) / "crates" / "core" / "src"
            core.mkdir(parents=True)
            (core / "lib.rs").write_text(
                'crate::i18n::text(\n  "По-русски",\n  "In English",\n);\n',
                encoding="utf-8",
            )
            rows, _ = module.scan(Path(directory), classify_localized_pairs=True)
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["classification"], "localized-pair")


if __name__ == "__main__":
    unittest.main()

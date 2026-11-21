import importlib.util
import unittest
from pathlib import Path

MODULE_PATH = Path(__file__).resolve().parents[1] / 'benchmark_autovacuum.py'
spec = importlib.util.spec_from_file_location('pgx_autovacuum_benchmark', MODULE_PATH)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class BenchmarkExtractionTests(unittest.TestCase):
    def test_extracts_exact_four_collector_sections(self):
        sql = module.extract_sql_constants(module.SQL_SOURCE.read_text(encoding='utf8'))
        self.assertEqual(tuple(sql), module.QUERIES)
        self.assertIn('pg_total_relation_size', sql['TABLES_SQL'])
        self.assertIn('pg_stat_user_tables', sql['TABLES_SQL'])

    def test_missing_section_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'Missing collector SQL'):
            module.extract_sql_constants('')


if __name__ == '__main__':
    unittest.main()

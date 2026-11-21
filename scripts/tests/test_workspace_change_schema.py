import re
import sqlite3
import unittest
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[2] / 'crates/core/src/workspace.rs'


class WorkspaceSchemaTests(unittest.TestCase):
    def test_change_journal_foreign_keys_and_required_fields(self):
        text = SOURCE.read_text(encoding='utf8')
        opening = text.index('impl WorkspaceStore {')
        match = re.search(r'\.execute_batch\(\s*r#"(.*?)"#', text[opening:], re.DOTALL)
        self.assertIsNotNone(match)
        conn = sqlite3.connect(':memory:')
        conn.executescript(match.group(1))
        columns = {row[1] for row in conn.execute('PRAGMA table_info(workspace_changes)')}
        for key in ('target_json', 'intervention_id', 'case_id', 'status', 'before_json', 'after_json'):
            self.assertIn(key, columns)
        conn.execute("INSERT INTO workspace_changes(connection_id,kind,parameter,before_json,after_json,status,started_at_ms,target_json) VALUES (1,'autovacuum_policy','autovacuum_vacuum_scale_factor','0.2','0.1','requested',100,'{}')")
        with self.assertRaises(sqlite3.IntegrityError):
            conn.execute("INSERT INTO workspace_changes(connection_id,kind,parameter,before_json,after_json,status,started_at_ms) VALUES (1,'xid_policy','x','1','2','successful',100)")
        self.assertEqual(conn.execute('PRAGMA foreign_key_check').fetchall(), [])
        conn.close()

    def test_target_column_migration_is_idempotent(self):
        text = SOURCE.read_text(encoding='utf8')
        self.assertIn('PRAGMA table_info(workspace_changes)', text)
        self.assertIn('workspace-change-target-v1', text)
        self.assertIn('ADD COLUMN target_json TEXT NOT NULL', text)


if __name__ == '__main__':
    unittest.main()

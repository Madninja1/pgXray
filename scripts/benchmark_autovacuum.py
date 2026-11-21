#!/usr/bin/env python3
"""Opt-in read-only EXPLAIN ANALYZE benchmark for PGX-011.

Requires psql/libpq credentials supplied by PG* environment variables.
Does not collect Vue, SQLite or Rust RAM metrics; measure those separately.
"""
from __future__ import annotations

import argparse
import json
import re
import statistics
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SQL_SOURCE = ROOT / 'crates/core/src/autovacuum/queries.rs'
QUERIES = ('OVERVIEW_SQL', 'TABLES_SQL', 'VACUUM_PROGRESS_SQL', 'ANALYZE_PROGRESS_SQL')


def extract_sql_constants(content: str) -> dict[str, str]:
    result = {}
    for name in QUERIES:
        match = re.search(r'pub\(super\) const ' + re.escape(name)
                          + r':\s*&str\s*=\s*r#"(.*?)"#;', content, re.DOTALL)
        if match is None:
            raise ValueError(f'Missing collector SQL: {name}')
        result[name] = match.group(1).strip()
    return result


def execute_once(sql: str, timeout_ms: int) -> dict:
    # Every process has its own read-only transaction and server-side timeout.
    request = (f'BEGIN READ ONLY;\nSET LOCAL statement_timeout = {timeout_ms};\n'
               f'EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {sql}\nROLLBACK;\n')
    completed = subprocess.run(
        ['psql', '-X', '-q', '-A', '-t', '--set=ON_ERROR_STOP=1'],
        input=request, capture_output=True, text=True, timeout=(timeout_ms / 1000) + 15,
        check=False,
    )
    if completed.returncode:
        # Avoid writing SQL or credentials to saved reports.
        raise RuntimeError(f'psql exited with {completed.returncode}; see local stderr for details')
    plans = json.loads(completed.stdout.strip())
    if not isinstance(plans, list) or not plans or 'Execution Time' not in plans[0]:
        raise ValueError('psql did not return expected EXPLAIN JSON')
    plan = plans[0]
    return {
        'planning_ms': plan.get('Planning Time'),
        'execution_ms': plan.get('Execution Time'),
        'shared_hit_blocks': plan.get('Plan', {}).get('Shared Hit Blocks'),
        'shared_read_blocks': plan.get('Plan', {}).get('Shared Read Blocks'),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description='Autovacuum collector SQL benchmark (opt-in)')
    parser.add_argument('--execute', action='store_true', help='Actually send EXPLAIN ANALYZE to PostgreSQL')
    parser.add_argument('--acknowledge-read-load', action='store_true', help='Accept read-only PostgreSQL load')
    parser.add_argument('--samples', type=int, default=3, choices=range(1, 11), metavar='1..10')
    parser.add_argument('--timeout-ms', type=int, default=30000)
    args = parser.parse_args()
    if not 1000 <= args.timeout_ms <= 120000:
        parser.error('--timeout-ms must be between 1000 and 120000')
    queries = extract_sql_constants(SQL_SOURCE.read_text(encoding='utf8'))
    print('Benchmark sections: ' + ', '.join(queries))
    if not args.execute:
        print('Dry-run. Use --execute --acknowledge-read-load on a test PostgreSQL. No queries sent.')
        return
    if not args.acknowledge_read_load:
        parser.error('--execute requires --acknowledge-read-load')
    # Nothing is changed on PostgreSQL, but EXPLAIN ANALYZE executes SELECTs.
    results = {}
    for name, sql in queries.items():
        samples = [execute_once(sql, args.timeout_ms) for _ in range(args.samples)]
        elapsed = [s['execution_ms'] for s in samples]
        results[name] = {
            'samples': samples,
            'execution_ms_min': min(elapsed),
            'execution_ms_median': statistics.median(elapsed),
            'execution_ms_max': max(elapsed),
        }
    print(json.dumps({'read_only': True, 'sample_count': args.samples,
                      'timeout_ms': args.timeout_ms, 'results': results},
                     ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()

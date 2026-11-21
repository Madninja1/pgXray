#!/usr/bin/env python3
"""Heuristic, non-destructive inventory of Russian text candidates in pgXray sources."""
from __future__ import annotations
import argparse
import csv
import json
import re
from collections import Counter
from pathlib import Path

CYRILLIC = re.compile(r'[А-Яа-яЁё]')
LITERAL = re.compile(r'''(?P<quote>["'`])(?P<value>(?:\\.|(?!\1)[^\n])*?)(?P=quote)''')
RUST_LOCALIZED_PAIR = re.compile(
    r'crate::i18n::text\(\s*"(?P<ru>(?:\\.|[^"\\])*)"\s*,\s*"(?:\\.|[^"\\])*"\s*,?\s*\)',
    re.S,
)
HTML_TEXT = re.compile(r'>(?P<value>[^<>]+)<', re.S)
ATTR = re.compile(r'''(?:placeholder|title|aria-label|alt|label|description)\s*=\s*(?P<quote>["'])(?P<value>.*?)(?P=quote)''', re.I | re.S)
ROOTS = [
    ('vue', 'ui-vue/src', {'.vue', '.ts', '.js'}),
    ('rust', 'crates', {'.rs'}),
    ('rust', 'src-tauri/src', {'.rs'}),
    ('legacy', 'ui', {'.html', '.js'}),
]

def classify(layer: str, kind: str, value: str) -> str:
    if layer == 'legacy': return 'legacy-review'
    if layer == 'rust': return 'backend-review'
    if '${' in value or '{{' in value: return 'dynamic-review'
    if kind in ('html-text', 'attribute'): return 'ui-candidate'
    return 'code-review'

def scan(repo: Path, include_legacy: bool = False, classify_localized_pairs: bool = False):
    rows = []
    scanned = Counter()
    for layer, rel, suffixes in ROOTS:
        if layer == 'legacy' and not include_legacy:
            continue
        folder = repo / rel
        if not folder.is_dir(): continue
        for path in sorted(folder.rglob('*')):
            if not path.is_file() or path.suffix not in suffixes: continue
            if any(part in ('node_modules', 'vendor', 'dist', 'target', 'gen') for part in path.parts): continue
            scanned[layer] += 1
            content = path.read_text(encoding='utf-8', errors='replace')
            paired_offsets = set()
            if layer == 'rust' and classify_localized_pairs:
                paired_offsets = {m.start('ru') for m in RUST_LOCALIZED_PAIR.finditer(content)}
            patterns = [('literal', LITERAL)]
            if path.suffix in ('.vue', '.html'):
                patterns += [('html-text', HTML_TEXT), ('attribute', ATTR)]
            for kind, pattern in patterns:
                for m in pattern.finditer(content):
                    value = ' '.join(m.group('value').strip().split())
                    if layer == 'rust' and kind == 'literal' and m.group('quote') != '"': continue
                    if not value or not CYRILLIC.search(value): continue
                    if kind == 'html-text' and value.startswith(('//', '/*', '*', '<!--')): continue
                    if kind == 'literal' and path.suffix in ('.vue', '.html'):
                        prefix = content[max(0, m.start()-40):m.start()]
                        if re.search(r'(?:placeholder|title|aria-label|alt|label|description)\s*=\s*$', prefix, re.I): continue
                    if len(value) > 1000: value = value[:1000] + '…'
                    offset = m.start('value')
                    rows.append({'layer':layer, 'file':path.relative_to(repo).as_posix(),
                                 'line':content.count('\n', 0, offset)+1, 'kind':kind,
                                 'classification': ('localized-pair' if offset in paired_offsets
                                     else classify(layer, kind, value)), 'text':value})
    rows.sort(key=lambda r: (r['file'], r['line'], r['kind']))
    return rows, scanned

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--output', type=Path, default=Path('i18n-audit'))
    parser.add_argument('--classify-localized-pairs', action='store_true',
                        help='Mark known Rust i18n::text(RU, EN) strings separately from review candidates')
    parser.add_argument('--include-legacy', action='store_true',
                        help='Include old ui/ HTML and JS sources in the inventory')
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows, scanned = scan(args.repo.resolve(), include_legacy=args.include_legacy,
                         classify_localized_pairs=args.classify_localized_pairs)
    with (out/'candidates.csv').open('w', newline='', encoding='utf-8-sig') as f:
        writer = csv.DictWriter(f, fieldnames=['layer','file','line','kind','classification','text'])
        writer.writeheader()
        writer.writerows(rows)
    (out/'candidates.json').write_text(json.dumps(rows, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    by_file = Counter(r['file'] for r in rows)
    summary = {'files_scanned':dict(scanned), 'candidate_occurrences':len(rows),
               'unique_texts':len(set(r['text'] for r in rows)), 'files_with_hits':len(by_file),
               'by_classification':dict(Counter(r['classification'] for r in rows)),
               'top_files':[{'file':p,'occurrences':n} for p,n in by_file.most_common(20)]}
    (out/'summary.json').write_text(json.dumps(summary,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(summary,ensure_ascii=False,indent=2))
    print('NOTE: candidates are heuristic, not all are actual UI strings; review before migration.')

if __name__ == '__main__': main()

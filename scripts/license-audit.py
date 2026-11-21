#!/usr/bin/env python3
"""Read-only license inventory for pgXray. Does not certify redistribution rights.

Run from the repo root: python3 scripts/license-audit.py
Optional: python3 scripts/license-audit.py --output /tmp/pgxray-licenses.md

Cargo package license metadata requires a local Cargo registry cache. npm package
license metadata is read from installed node_modules; no packages are downloaded.
"""
import argparse
import collections
import json
from pathlib import Path
import subprocess
import tomllib

VENDORS = [
    ('PEV2', 'ui/vendor/pev2/PEV2-LICENSE'),
    ('Vue (PEV2 asset)', 'ui/vendor/pev2/VUE-LICENSE'),
    ('Bootstrap (PEV2 asset)', 'ui/vendor/pev2/BOOTSTRAP-LICENSE'),
    ('xterm.js', 'ui/vendor/xterm/LICENSE.xterm'),
    ('xterm fit addon', 'ui/vendor/xterm/LICENSE.addon-fit'),
]


def run(args, root):
    try:
        r = subprocess.run(args, cwd=root, capture_output=True, text=True,
                           timeout=120, check=False)
        return (r.returncode, r.stdout)
    except (OSError, subprocess.TimeoutExpired):
        return (1, '')


def fmt_table(rows):
    if not rows:
        return '_No entries._\n'
    return '\n'.join('| ' + ' | '.join(str(x).replace('|', '\\|').replace('\n', ' ') for x in row) + ' |' for row in rows) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', default='.', help='Root of pgXray checkout')
    parser.add_argument('--output', help='Write generated Markdown report to this file')
    opts = parser.parse_args()
    root = Path(opts.repo).resolve()
    lock = tomllib.loads((root / 'Cargo.lock').read_text(encoding='utf-8'))
    rust_locked = {(p['name'], p['version']) for p in lock['package']}
    npm_lock = json.loads((root / 'ui-vue/package-lock.json').read_text(encoding='utf-8'))
    npm_locked = [(k.removeprefix('node_modules/'), v.get('version', 'workspace'))
                  for k, v in npm_lock['packages'].items() if k]

    lines = ['# pgXray dependency license inventory', '',
             'Generated locally, read-only. **This is not a legal clearance or a complete distribution audit.**', '',
             f'- Rust lockfile package entries: **{len(lock["package"])}**',
             f'- npm lockfile dependency entries (excluding root): **{len(npm_locked)}**', '']
    lines += ['## Bundled assets', '', '| Component | License file | Present |', '| --- | --- | --- |']
    for name, location in VENDORS:
        lines.append(f'| {name} | `{location}` | {"yes" if (root / location).is_file() else "NO"} |')

    lines += ['', '## Rust package license expressions', '']
    code, stdout = run(['cargo', 'metadata', '--offline', '--locked', '--format-version', '1'], root)
    if code == 0:
        try:
            packages = json.loads(stdout)['packages']
            matched = {(p['name'], p['version']): p for p in packages}
            rows = [('Package', 'Version', 'License expression', 'License file')]
            for name, version in sorted(rust_locked):
                p = matched.get((name, version))
                rows.append((name, version, (p or {}).get('license') or 'UNKNOWN',
                             'yes' if (p or {}).get('license_file') else 'no'))
            lines += ['Cargo metadata available (offline). `UNKNOWN` requires review.', '',
                      fmt_table(rows)]
        except (ValueError, KeyError, TypeError):
            lines += ['Cargo metadata returned an unreadable response. Rust licenses: **NOT VERIFIED**.', '']
    else:
        lines += ['`cargo metadata --offline --locked` unavailable or incomplete.',
                  f'**Rust licenses NOT VERIFIED for {len(rust_locked)} locked packages.**',
                  'Install/use the Rust toolchain and fetch the already locked dependencies locally, then rerun.', '']

    lines += ['## npm installed package license expressions', '']
    rows = [('Package', 'Version', 'License in installed package.json')]
    unknown = 0
    for name, version in sorted(npm_locked):
        manifest = root / 'ui-vue/node_modules' / name / 'package.json'
        try:
            value = json.loads(manifest.read_text(encoding='utf-8')).get('license') or 'UNKNOWN'
        except (OSError, ValueError):
            value = 'NOT INSTALLED'
        if value in ('UNKNOWN', 'NOT INSTALLED'):
            unknown += 1
        rows.append((name, version, value))
    lines += [f'Entries without verifiable local license metadata: **{unknown}/{len(npm_locked)}**.',
              'The lockfile does not record licenses; installed package manifests are supplementary evidence.',
              'Do not assume every installed dependency is shipped with the final Vue bundle.', '', fmt_table(rows)]

    lines += ['## Manual review before publishing binaries', '',
              '- Review UNKNOWN/non-standard/dual/copyleft licenses against upstream license texts.',
              '- Inspect native libraries and platform WebView requirements in actual packaging.',
              '- Include required notices and license texts in the distributed artifact.',
              '- Confirm original-code publishing rights and scan the complete Git history for secrets.',
              '- Repeat after updating lockfiles or vendor bundles.', '']
    report = '\n'.join(lines)
    if opts.output:
        path = Path(opts.output).expanduser()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(report, encoding='utf-8')
        print(f'Wrote {path}')
    else:
        print(report)


if __name__ == '__main__':
    main()

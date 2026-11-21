import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const source = fs.readFileSync(path.join(root, 'ui/js/xid.js'), 'utf8');
const start = source.indexOf('function postgresDocsUrl(path)');
const end = source.indexOf('function renderXidMethodology()', start);
assert.ok(start >= 0 && end > start, 'XID source renderer should be present');
const renderer = source.slice(start, end);

function element(tag) {
  return {
    tag,
    children: [],
    textContent: '',
    href: '',
    rel: '',
    target: '',
    replaceChildren() { this.children = []; },
    append(...nodes) { this.children.push(...nodes); },
  };
}

for (const version of ['18', 'current']) {
  test(`XID documentation sources link to PostgreSQL ${version} without opener access`, () => {
    const list = element('div');
    const versionLabel = element('p');
    const context = {
      xidReference: {
        sources: [{
          title: 'XID documentation',
          purpose: 'PostgreSQL manual',
          path: 'functions-info.html',
        }],
      },
      postgresDocsVersion: () => version,
      byId: (id) => id === 'xid-source-list' ? list : versionLabel,
      document: { createElement: element },
    };
    vm.runInNewContext(`${renderer}\nrenderXidSources();`, context);
    assert.equal(list.children.length, 1);
    const [title, purpose, link] = list.children[0].children;
    assert.equal(title.textContent, 'XID documentation');
    assert.equal(purpose.textContent, 'PostgreSQL manual');
    assert.equal(link.tag, 'a');
    assert.equal(link.href, `https://www.postgresql.org/docs/${version}/functions-info.html`);
    assert.equal(link.textContent, link.href);
    assert.equal(link.target, '_blank');
    assert.equal(link.rel, 'noopener noreferrer');
  });
}

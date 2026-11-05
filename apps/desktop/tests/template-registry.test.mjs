import assert from 'node:assert/strict';
import test from 'node:test';
import {
  expandNoteTemplateText,
  readNoteTemplateRegistry,
  writeNoteTemplateRegistry,
} from '../src/organization/template-registry.ts';
import { createTemplatedNoteDocument } from '../src/organization/template-document.ts';
import { readFrontmatter, readObjectType } from '../src/organization/object-types.ts';

const t = (zh) => zh;
const now = new Date(2026, 9, 25, 9, 7, 0);
const customType = {
  id: 'story-arc',
  labelZh: '故事线',
  labelEn: 'Story arc',
  pluralZh: '故事线',
  pluralEn: 'Story arcs',
  descriptionZh: '追踪变化。',
  descriptionEn: 'Track change.',
  icon: 'custom',
  tone: 'violet',
  aliases: ['arc'],
  fields: [
    {
      key: 'stage',
      labelZh: '阶段',
      labelEn: 'Stage',
      kind: 'select',
      defaultValue: 'setup',
      options: [{ value: 'setup', zh: '铺垫', en: 'Setup' }],
    },
  ],
  sections: [{ zh: '概览', en: 'Overview' }],
};

function template(overrides = {}) {
  return {
    id: 'arc-outline',
    nameZh: '故事线大纲',
    nameEn: 'Arc outline',
    descriptionZh: '从三段式结构开始。',
    descriptionEn: 'Start with a three-act structure.',
    typeId: 'story-arc',
    titleZh: '故事线 · {{date}}',
    titleEn: 'Story arc · {{date}}',
    bodyZh: '## 概览\n\n- [ ] 起点\n- [ ] 转折\n- [ ] 回收',
    bodyEn: '## Overview\n\n- [ ] Setup\n- [ ] Turn\n- [ ] Payoff',
    ...overrides,
  };
}

test('note template registry is versioned, workspace-scoped and normalized', () => {
  const one = writeNoteTemplateRegistry(null, 'workspace-a', [template()]);
  const parsed = JSON.parse(one);
  assert.equal(parsed.schema, 1);
  assert.equal(parsed.workspaces['workspace-a'][0].id, 'arc-outline');
  assert.deepEqual(readNoteTemplateRegistry(one, 'workspace-b').templates, []);
  const loaded = readNoteTemplateRegistry(one, 'workspace-a');
  assert.equal(loaded.error, null);
  assert.equal(loaded.templates[0].typeId, 'story-arc');
  const two = writeNoteTemplateRegistry(one, 'workspace-b', [template({ id: 'meeting-template' })]);
  assert.equal(JSON.parse(two).workspaces['workspace-a'][0].id, 'arc-outline');
  assert.equal(JSON.parse(two).workspaces['workspace-b'][0].id, 'meeting-template');
});

test('template validation rejects duplicate ids, invalid types, bad schema and oversized bodies', () => {
  assert.throws(
    () => writeNoteTemplateRegistry(null, 'workspace-a', [template(), template()]),
    /Duplicate template id/,
  );
  assert.throws(
    () => writeNoteTemplateRegistry(null, 'workspace-a', [template({ typeId: 'bad type' })]),
    /invalid object type id/,
  );
  assert.match(
    readNoteTemplateRegistry('{"schema":2,"workspaces":{}}', 'workspace-a').error,
    /schema is unsupported/i,
  );
  assert.throws(
    () =>
      writeNoteTemplateRegistry(null, 'workspace-a', [template({ bodyZh: 'x'.repeat(20_001) })]),
    /exceeds 20000 characters/,
  );
});

test('template expansion supports date, time and type while preserving unknown tokens', () => {
  assert.equal(
    expandNoteTemplateText('{{date}} {{time}} {{type}} {{unknown}}', {
      typeLabel: '故事线',
      now,
    }),
    '2026-10-25 09:07 故事线 {{unknown}}',
  );
});

test('typed template creation keeps frontmatter defaults, parsed markdown and provenance', () => {
  const created = createTemplatedNoteDocument(template(), t, 'zh-CN', [customType], now);
  assert.equal(created.title, '故事线 · 2026-10-25');
  assert.equal(readObjectType(created.document, [customType]).definition.id, 'story-arc');
  assert.equal(readFrontmatter(created.document).values.type, 'story-arc');
  assert.equal(readFrontmatter(created.document).values.stage, 'setup');
  assert.equal(readFrontmatter(created.document).values.template, 'arc-outline');
  assert.ok(
    created.document.content.some(
      (node) => node.type === 'heading' && node.content?.[0]?.text === '概览',
    ),
  );
  assert.ok(created.document.content.some((node) => node.type === 'taskList'));
  assert.deepEqual(created.preserved, []);
});

test('empty template body falls back to object type sections and unknown types fail loudly', () => {
  const fallback = createTemplatedNoteDocument(
    template({ bodyZh: '', bodyEn: '' }),
    t,
    'zh-CN',
    [customType],
    now,
  );
  assert.ok(
    fallback.document.content.some(
      (node) => node.type === 'heading' && node.content?.[0]?.text === '概览',
    ),
  );
  assert.throws(
    () =>
      createTemplatedNoteDocument(
        template({ typeId: 'missing-type' }),
        t,
        'zh-CN',
        [customType],
        now,
      ),
    /unavailable/,
  );
});

test('plain template without an object type creates frontmatter provenance only', () => {
  const created = createTemplatedNoteDocument(
    template({ typeId: null, titleZh: '每日记录 {{date}}', bodyZh: '今天观察到了什么？' }),
    t,
    'zh-CN',
    [],
    now,
  );
  assert.equal(created.title, '每日记录 2026-10-25');
  assert.equal(readFrontmatter(created.document).values.type, undefined);
  assert.equal(readFrontmatter(created.document).values.template, 'arc-outline');
  assert.ok(
    created.document.content.some(
      (node) => node.type === 'paragraph' && node.content?.[0]?.text === '今天观察到了什么？',
    ),
  );
});

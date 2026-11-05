import assert from 'node:assert/strict';
import test from 'node:test';
import {
  OBJECT_TYPES,
  createObjectTypeDocument,
  objectTypeDefaultTitle,
  objectTypeDefinition,
  readFrontmatter,
  readObjectRelations,
  readObjectTypeRegistry,
  writeObjectTypeRegistry,
  readFrontmatterText,
  readObjectType,
  setFrontmatterRelations,
  setFrontmatterValue,
  setObjectType,
} from '../src/organization/object-types.ts';

const t = (zh) => zh;

test('built-in object types have stable unique identities and fields', () => {
  const ids = OBJECT_TYPES.map((type) => type.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.ok(ids.includes('character'));
  assert.ok(ids.includes('person'));
  assert.ok(ids.includes('book'));
  assert.ok(ids.includes('project'));
  for (const type of OBJECT_TYPES) {
    assert.ok(type.labelZh);
    assert.ok(type.labelEn);
    assert.equal(new Set(type.fields.map((field) => field.key)).size, type.fields.length);
  }
});

test('aliases, case, custom values, and missing frontmatter resolve predictably', () => {
  assert.equal(objectTypeDefinition('Characters').id, 'character');
  assert.equal(objectTypeDefinition('电影').id, 'film');
  assert.equal(objectTypeDefinition('unknown-type').id, 'custom:unknown-type');
  const empty = { type: 'doc', content: [] };
  assert.equal(readObjectType(empty).definition.id, 'note');
  assert.equal(readObjectType(empty).rawType, null);
});

test('typed creation writes frontmatter first and starts from an editable template', () => {
  const document = createObjectTypeDocument('character', t);
  assert.equal(document.content[0].type, 'frontmatter');
  assert.match(document.content[0].attrs.source, /^type: character$/m);
  assert.equal(readObjectType(document).definition.id, 'character');
  assert.ok(
    document.content.some((node) => node.type === 'heading' && node.content?.[0]?.text === '概览'),
  );
  assert.equal(objectTypeDefaultTitle('book', t), '未命名书籍');
  const plain = createObjectTypeDocument('note', t);
  assert.equal(plain.content[0].type, 'paragraph');
});

test('property updates preserve unknown YAML keys, types, comments, and round-trip values', () => {
  const document = {
    type: 'doc',
    content: [
      {
        type: 'frontmatter',
        attrs: {
          id: 'frontmatter',
          source: [
            '# kept comment',
            'type: book',
            'author: Ursula K. Le Guin',
            'custom_nested:',
            '  keep: true',
          ].join('\n'),
        },
      },
      { type: 'paragraph', attrs: { id: 'body' } },
    ],
  };
  const changed = setFrontmatterValue(document, 'summary', 'A "quoted" summary: with colon');
  assert.equal(readFrontmatterText(changed, 'summary'), 'A "quoted" summary: with colon');
  const values = readFrontmatter(changed).values;
  assert.equal(values.author, 'Ursula K. Le Guin');
  assert.deepEqual(values.custom_nested, { keep: true });
  assert.match(changed.content[0].attrs.source, /# kept comment/);
  assert.equal(document.content[0].attrs.source.includes('summary'), false);
});

test('changing type preserves old typed values instead of destructively rewriting them', () => {
  const original = createObjectTypeDocument('book', t);
  const changed = setObjectType(original, 'film');
  assert.equal(readObjectType(changed).definition.id, 'film');
  assert.equal(readFrontmatter(changed).values.status, 'want');
  assert.equal(readFrontmatter(changed).values.type, 'film');
});

test('invalid or non-mapping frontmatter is visible and blocked from mutation', () => {
  const invalid = {
    type: 'doc',
    content: [
      { type: 'frontmatter', attrs: { id: 'frontmatter', source: 'type: [broken' } },
      { type: 'paragraph', attrs: { id: 'body' } },
    ],
  };
  assert.match(readFrontmatter(invalid).error, /flow sequence|unexpected end/i);
  assert.throws(() => setFrontmatterValue(invalid, 'summary', 'x'));
  const list = {
    type: 'doc',
    content: [{ type: 'frontmatter', attrs: { id: 'frontmatter', source: '- one\n- two' } }],
  };
  assert.match(readFrontmatter(list).error, /mapping/i);
  assert.throws(() => setFrontmatterValue(list, 'summary', 'x'), /mapping/i);
});

test('clearing a field removes only that key and keeps the frontmatter source', () => {
  const original = createObjectTypeDocument('person', t);
  const changed = setFrontmatterValue(original, 'summary', 'Ada');
  const cleared = setFrontmatterValue(changed, 'summary', '   ');
  assert.equal(readFrontmatter(cleared).values.summary, undefined);
  assert.equal(readObjectType(cleared).definition.id, 'person');
  assert.equal(cleared.content[0].type, 'frontmatter');
});

function customType(overrides = {}) {
  return {
    id: 'story-arc',
    labelZh: '故事线',
    labelEn: 'Story arc',
    pluralZh: '故事线',
    pluralEn: 'Story arcs',
    descriptionZh: '追踪角色与情节在一段故事中的变化。',
    descriptionEn: 'Track how characters and plots change across a story.',
    icon: 'custom',
    tone: 'violet',
    aliases: ['arc', '故事线'],
    fields: [
      {
        key: 'stage',
        labelZh: '阶段',
        labelEn: 'Stage',
        kind: 'select',
        defaultValue: 'setup',
        options: [
          { value: 'setup', zh: '铺垫', en: 'Setup' },
          { value: 'payoff', zh: '回收', en: 'Payoff' },
        ],
      },
    ],
    sections: [{ zh: '节拍', en: 'Beats' }],
    ...overrides,
  };
}

test('custom object type registry is versioned, workspace-scoped, and normalized', () => {
  const first = customType();
  const json = writeObjectTypeRegistry('{"schema":1,"workspaces":{}}', 'workspace-a', [first]);
  const stored = JSON.parse(json);
  assert.equal(stored.schema, 1);
  assert.equal(stored.workspaces['workspace-a'][0].id, 'story-arc');
  assert.deepEqual(readObjectTypeRegistry(json, 'workspace-b').types, []);
  const loaded = readObjectTypeRegistry(json, 'workspace-a');
  assert.equal(loaded.error, null);
  assert.equal(loaded.types[0].fields[0].defaultValue, 'setup');
  const second = writeObjectTypeRegistry(json, 'workspace-b', [
    customType({ id: 'beat-sheet', aliases: ['beats'] }),
  ]);
  assert.equal(JSON.parse(second).workspaces['workspace-a'][0].id, 'story-arc');
  assert.equal(JSON.parse(second).workspaces['workspace-b'][0].id, 'beat-sheet');
});

test('custom object types resolve aliases and create typed templates', () => {
  const custom = customType();
  assert.equal(objectTypeDefinition('ARC', [custom]).id, 'story-arc');
  assert.equal(objectTypeDefaultTitle('story-arc', t, [custom]), '未命名故事线');
  const document = createObjectTypeDocument('story-arc', t, [custom]);
  assert.match(document.content[0].attrs.source, /^type: story-arc$/m);
  assert.match(document.content[0].attrs.source, /^stage: setup$/m);
  assert.ok(
    document.content.some((node) => node.type === 'heading' && node.content?.[0]?.text === '节拍'),
  );
  assert.equal(readObjectType(document, [custom]).definition.labelZh, '故事线');
});

test('custom object type identifiers and aliases cannot collide case-insensitively', () => {
  assert.throws(
    () => writeObjectTypeRegistry(null, 'workspace-a', [customType({ id: 'character' })]),
    /reserved or in use/,
  );
  assert.throws(
    () => writeObjectTypeRegistry(null, 'workspace-a', [customType({ aliases: ['Characters'] })]),
    /reserved or in use/,
  );
  assert.throws(
    () =>
      writeObjectTypeRegistry(null, 'workspace-a', [
        customType(),
        customType({ id: 'scene-card', aliases: ['ARC'] }),
      ]),
    /reserved or in use/,
  );
  assert.throws(
    () => writeObjectTypeRegistry(null, 'workspace-a', [customType({ aliases: ['arc', 'Arc'] })]),
    /duplicate alias/,
  );
});

test('custom object type validation rejects unsupported shapes and bad defaults', () => {
  assert.match(readObjectTypeRegistry('{', 'workspace-a').error, /JSON|Unexpected/i);
  assert.match(
    readObjectTypeRegistry('{"schema":2,"workspaces":{}}', 'workspace-a').error,
    /schema is unsupported/i,
  );
  assert.throws(
    () =>
      writeObjectTypeRegistry(null, 'workspace-a', [
        customType({ fields: [{ ...customType().fields[0], defaultValue: 'missing' }] }),
      ]),
    /default value is not one of its options/,
  );
  assert.throws(
    () => writeObjectTypeRegistry(null, 'workspace-a', [customType({ fields: [] }), customType()]),
    /already reserved or in use/,
  );
});

test('relation fields round-trip stable note references and reject malformed values', () => {
  const noteId = '11111111-1111-4111-8111-111111111111';
  const otherId = '22222222-2222-4222-8222-222222222222';
  const original = createObjectTypeDocument('project', t);
  const changed = setFrontmatterRelations(original, 'related', [
    { kind: 'note', id: noteId, title: 'Launch plan' },
    { kind: 'note', id: otherId, title: 'Risk review' },
  ]);
  assert.deepEqual(readObjectRelations(changed, 'related').values, [
    { kind: 'note', id: noteId, title: 'Launch plan' },
    { kind: 'note', id: otherId, title: 'Risk review' },
  ]);
  assert.equal(readObjectType(changed).definition.id, 'project');
  assert.throws(
    () =>
      setFrontmatterRelations(original, 'related', [
        { kind: 'note', id: noteId, title: 'One' },
        { kind: 'note', id: noteId, title: 'Again' },
      ]),
    /cannot repeat/,
  );
  const malformed = {
    type: 'doc',
    content: [
      {
        type: 'frontmatter',
        attrs: { id: 'frontmatter', source: 'related:\n  - title: Missing ID' },
      },
    ],
  };
  assert.match(readObjectRelations(malformed, 'related').error, /note reference|invalid note ID/i);
  const cleared = setFrontmatterRelations(changed, 'related', []);
  assert.equal(readFrontmatter(cleared).values.related, undefined);
});

test('object type registry accepts relation fields but rejects relation defaults or options', () => {
  const relation = {
    key: 'related',
    labelZh: '相关笔记',
    labelEn: 'Related notes',
    kind: 'relation',
  };
  const json = writeObjectTypeRegistry(null, 'workspace-a', [customType({ fields: [relation] })]);
  assert.equal(readObjectTypeRegistry(json, 'workspace-a').types[0].fields[0].kind, 'relation');
  assert.throws(
    () =>
      writeObjectTypeRegistry(null, 'workspace-a', [
        customType({ fields: [{ ...relation, defaultValue: 'x' }] }),
      ]),
    /cannot have a default value/,
  );
  assert.throws(
    () =>
      writeObjectTypeRegistry(null, 'workspace-a', [
        customType({ fields: [{ ...relation, options: [{ value: 'x', zh: 'X', en: 'X' }] }] }),
      ]),
    /cannot have options/,
  );
});

test('relation fields can restrict allowed object types without affecting other field kinds', () => {
  const relation = {
    key: 'related',
    labelZh: '相关笔记',
    labelEn: 'Related notes',
    kind: 'relation',
    allowedTypeIds: ['project', 'idea'],
  };
  const json = writeObjectTypeRegistry(null, 'workspace-a', [customType({ fields: [relation] })]);
  assert.deepEqual(readObjectTypeRegistry(json, 'workspace-a').types[0].fields[0].allowedTypeIds, [
    'project',
    'idea',
  ]);
  assert.throws(
    () =>
      writeObjectTypeRegistry(null, 'workspace-a', [
        customType({ fields: [{ ...relation, allowedTypeIds: ['project', 'project'] }] }),
      ]),
    /invalid or duplicate allowed type/,
  );
  assert.throws(
    () =>
      writeObjectTypeRegistry(null, 'workspace-a', [
        customType({
          fields: [
            {
              key: 'status',
              labelZh: '状态',
              labelEn: 'Status',
              kind: 'text',
              allowedTypeIds: ['project'],
            },
          ],
        }),
      ]),
    /Only relation fields can restrict object types/,
  );
});

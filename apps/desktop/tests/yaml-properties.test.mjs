import assert from 'node:assert/strict';
import test from 'node:test';
import { objectTypeChangeAllowed, objectTypeValue } from '../src/rich/yaml-properties.ts';

const allowed = ['embedded', 'research', 'project', 'note'];

test('reads the controlled object type from top-level frontmatter only', () => {
  assert.equal(
    objectTypeValue('type: embedded\nsummary: 中断向量表与 RTOS 任务调度\n'),
    'embedded',
  );
  assert.equal(objectTypeValue('object_type: research\nsummary: 访谈编码与证据链\n'), 'research');
  assert.equal(objectTypeValue('metadata:\n  type: embedded\nsummary: 只保留嵌套结构\n'), null);
  assert.equal(objectTypeValue('summary: 没有声明对象类型\n'), null);
});

test('preserves a legacy unknown type when unrelated properties change', () => {
  const before = 'type: organization\nsummary: 嵌入式团队协作\n';
  const after = 'type: organization\nsummary: 嵌入式团队协作与评审记录\n';
  assert.equal(objectTypeChangeAllowed(before, after, allowed), true);
});

test('rejects a new free-form type from raw frontmatter', () => {
  assert.equal(
    objectTypeChangeAllowed(
      'type: embedded\nsummary: 中断向量表\n',
      'type: free-form\nsummary: 中断向量表\n',
      allowed,
    ),
    false,
  );
  assert.equal(
    objectTypeChangeAllowed('summary: 访谈编码\n', 'type: free-form\nsummary: 访谈编码\n', allowed),
    false,
  );
});

test('allows switching between registered object types and adding a known type', () => {
  assert.equal(
    objectTypeChangeAllowed(
      'type: embedded\nsummary: 中断向量表\n',
      'type: research\nsummary: 中断向量表\n',
      allowed,
    ),
    true,
  );
  assert.equal(
    objectTypeChangeAllowed(
      'summary: 个人研究问题\n',
      'type: research\nsummary: 个人研究问题\n',
      allowed,
    ),
    true,
  );
});
test('does not allow raw YAML to bypass type selection when the registry is unavailable', () => {
  assert.equal(
    objectTypeChangeAllowed(
      'type: note\nsummary: 保留旧类型\n',
      'type: free-form\nsummary: 保留旧类型\n',
      [],
    ),
    false,
  );
  assert.equal(
    objectTypeChangeAllowed(
      'type: note\nsummary: 保留旧类型\n',
      'type: note\nsummary: 仅修改摘要\n',
      [],
    ),
    true,
  );
});

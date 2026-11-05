import test from 'node:test';
import assert from 'node:assert/strict';
import { safeFilename, viewCsv } from '../src/organization/view-export.ts';

test('view CSV exports current metadata with escaped cells and an explicit lossy boundary', () => {
  const view = {
    name: 'Research, "board"',
    config: { custom_properties: ['due', 'priority'] },
  };
  const csv = viewCsv(view, [
    {
      note: {
        id: 'note-1',
        title: 'A "quoted" note',
        revision_id: 'r1',
        document_schema: 3,
        preview: 'line one, line two',
        trashed: false,
        updated_at: '2026-10-18T00:00:00Z',
        row_version: 1,
        inbox_state: 'pending',
        archived: false,
        search_enabled: true,
      },
      metadata: {
        id: 'note-1',
        kind: 'note',
        row_version: 1,
        archived: false,
        trashed: false,
        inbox_state: 'pending',
        search_enabled: true,
        tags: [{ id: 'tag-1', display_name: '研究', normalized_name: '研究' }],
      },
      properties: [
        {
          key: 'due',
          value: '2026-10-18',
          kind: 'date',
          truncated: true,
        },
      ],
      properties_truncated: true,
      properties_error: false,
    },
  ]);
  assert.ok(csv.startsWith('\uFEFF'));
  assert.match(csv, /"Research, ""board"""/);
  assert.match(csv, /"A ""quoted"" note"/);
  assert.match(csv, /"研究"/);
  assert.match(csv, /"line one, line two"/);
  assert.match(csv, /"property:due"/);
  assert.match(csv, /"2026-10-18…"/);
  assert.match(csv, /"properties=truncated\|due=truncated\|priority=missing"/);
  assert.equal(safeFilename('a/b:c*'), 'a-b-c-');
});

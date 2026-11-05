CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    current_revision INTEGER NOT NULL CHECK(current_revision > 0),
    anchor_id TEXT NOT NULL UNIQUE,
    trashed INTEGER NOT NULL DEFAULT 0 CHECK(trashed IN (0, 1)),
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE note_revisions (
    note_id TEXT NOT NULL REFERENCES notes(id),
    revision INTEGER NOT NULL,
    title TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK(schema_version = 1),
    document_json TEXT NOT NULL CHECK(json_valid(document_json)),
    created_at TEXT NOT NULL,
    PRIMARY KEY(note_id, revision)
) STRICT;

CREATE TABLE collections (id TEXT PRIMARY KEY, name TEXT NOT NULL) STRICT;
CREATE TABLE memberships (
    collection_id TEXT NOT NULL REFERENCES collections(id),
    note_id TEXT NOT NULL REFERENCES notes(id),
    PRIMARY KEY(collection_id, note_id)
) STRICT;
CREATE INDEX memberships_note ON memberships(note_id);

CREATE TABLE note_links (
    source_id TEXT NOT NULL REFERENCES notes(id),
    target_id TEXT NOT NULL REFERENCES notes(id),
    context TEXT NOT NULL,
    PRIMARY KEY(source_id, target_id)
) STRICT;
CREATE INDEX links_target ON note_links(target_id);

CREATE VIRTUAL TABLE note_fts USING fts5(note_id UNINDEXED, title_tokens, body_tokens);
PRAGMA application_id = 1347112275;
PRAGMA user_version = 1;

-- Frozen user rows for migration tests, authored against the released v1 schema.
-- JSON spacing and Unicode deliberately vary; migration must not reserialize.
PRAGMA foreign_keys = ON;
BEGIN;
INSERT INTO notes VALUES('11111111-1111-4111-8111-111111111111','迁移前的知识笔记',2,'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',0,'2026-09-30T10:00:00+00:00');
INSERT INTO note_revisions VALUES('11111111-1111-4111-8111-111111111111',1,'最初标题',1,'{"type": "doc", "content": [{"type": "paragraph", "attrs": {"id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}, "content": [{"type": "text", "text": "\u7b2c\u4e00\u7248\n\u4e2d\u6587\ud83d\ude42"}]}]}','2026-09-30T10:00:00+00:00');
INSERT INTO note_revisions VALUES('11111111-1111-4111-8111-111111111111',2,'迁移前的知识笔记',1,'{
  "type": "doc",
  "content": [
    {
      "type": "paragraph",
      "attrs": {
        "id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
      },
      "content": [
        {
          "type": "text",
          "text": "第二版：保留原始 JSON 字节\nRust SQLite"
        }
      ]
    }
  ]
}','2026-09-30T10:00:00+00:00');
INSERT INTO notes VALUES('22222222-2222-4222-8222-222222222222','参考来源',1,'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',0,'2026-09-30T10:00:00+00:00');
INSERT INTO note_revisions VALUES('22222222-2222-4222-8222-222222222222',1,'参考来源',1,'{"type": "doc", "content": [{"type": "paragraph", "attrs": {"id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"}, "content": [{"type": "text", "text": "\u5f15\u7528\u77e5\u8bc6\u7b14\u8bb0"}]}]}','2026-09-30T10:00:00+00:00');
INSERT INTO notes VALUES('33333333-3333-4333-8333-333333333333','已回收但不可遗失',1,'cccccccc-cccc-4ccc-8ccc-cccccccccccc',1,'2026-09-30T10:00:00+00:00');
INSERT INTO note_revisions VALUES('33333333-3333-4333-8333-333333333333',1,'已回收但不可遗失',1,'{"type": "doc", "content": [{"type": "paragraph", "attrs": {"id": "cccccccc-cccc-4ccc-8ccc-cccccccccccc"}, "content": [{"type": "text", "text": "\u56de\u6536\u7ad9\u6b63\u6587\ud83d\ude42"}]}]}','2026-09-30T10:00:00+00:00');
INSERT INTO collections VALUES('44444444-4444-4444-8444-444444444444','学习');
INSERT INTO collections VALUES('55555555-5555-4555-8555-555555555555','写作');
INSERT INTO memberships VALUES('44444444-4444-4444-8444-444444444444','11111111-1111-4111-8111-111111111111');
INSERT INTO memberships VALUES('55555555-5555-4555-8555-555555555555','11111111-1111-4111-8111-111111111111');
INSERT INTO memberships VALUES('44444444-4444-4444-8444-444444444444','33333333-3333-4333-8333-333333333333');
INSERT INTO note_links VALUES('22222222-2222-4222-8222-222222222222','11111111-1111-4111-8111-111111111111','原始引用上下文🙂');
INSERT INTO note_fts(note_id,title_tokens,body_tokens) VALUES('11111111-1111-4111-8111-111111111111','迁移 知识 笔记','第二版 保留 原始 json 字节 rust sqlite');
INSERT INTO note_fts(note_id,title_tokens,body_tokens) VALUES('22222222-2222-4222-8222-222222222222','参考 来源','引用 知识 笔记');
INSERT INTO note_fts(note_id,title_tokens,body_tokens) VALUES('33333333-3333-4333-8333-333333333333','回收 遗失','回收站 正文');
COMMIT;

-- Parsed book images are content-addressed blobs owned by one immutable parse.
-- The image is attached to a real source block so chapter navigation, focus and
-- reading-position restore continue to use the same stable block identity.
CREATE TABLE source_assets (
 parse_id TEXT NOT NULL REFERENCES source_parses(id),
 blob_hash TEXT NOT NULL REFERENCES blobs(hash),
 media_type TEXT NOT NULL,
 created_at TEXT NOT NULL,
 PRIMARY KEY(parse_id,blob_hash)
) STRICT;
CREATE INDEX source_assets_blob ON source_assets(blob_hash);
CREATE TABLE source_block_images (
 parse_id TEXT NOT NULL REFERENCES source_parses(id),
 block_id TEXT NOT NULL REFERENCES source_blocks(id),
 blob_hash TEXT NOT NULL,
 position INTEGER NOT NULL CHECK(position>=0),
 alt_text TEXT NOT NULL DEFAULT '',
 PRIMARY KEY(parse_id,block_id),
 FOREIGN KEY(parse_id,blob_hash) REFERENCES source_assets(parse_id,blob_hash)
) STRICT;
CREATE INDEX source_block_images_position ON source_block_images(parse_id,position);
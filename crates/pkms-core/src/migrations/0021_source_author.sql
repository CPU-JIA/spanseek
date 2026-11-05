-- FA-1 metadata needs an author. It is filled from the document's own metadata
-- when the parser finds one and stays NULL when the format carries none, so the
-- interface can say "not provided" instead of inventing a name. The bound matches
-- the display title bound, and the value is never derived from the file name.
ALTER TABLE sources ADD COLUMN author TEXT
    CHECK(author IS NULL OR (length(author) BETWEEN 1 AND 300));
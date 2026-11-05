# Synthetic retrieval and answer fixtures

These authored documents and labels contain no user or workspace material.
The development and reserved sets were fixed before model execution; the reserved
set is not claimed to be independently labelled or blind.

`quality_retrieval.rs`, `quality_answers.rs` and the answer budget example compile
these files directly. Keep them with the tests, independently of local evaluation
scripts. `manifest.json` records provenance, counts and exact SHA-256 values;
`.gitattributes` preserves their bytes. Updating a fixture requires reviewing its
labels and updating the corresponding fixed test hashes.

Live model tests remain opt-in and ignored until their explicit runtime,
credential and fresh-output requirements are met. Fixture inclusion does not
claim that a live provider or answer-quality evaluation has passed.

//! Per-library chunk settings and index rollback: defaults, versioning,
//! idempotency, validation and the profile swap that a new chunk cap causes.
use pkms_core::*;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn spec(tokens: u32, overlap: u32) -> EmbeddingProfileSpec {
    EmbeddingProfileSpec {
        endpoint: Some("https://example.invalid/v1".into()),
        model: "@cf/baai/bge-m3".into(),
        revision: "cloudflare-hosted-bge-m3-v1".into(),
        dimensions: 1024,
        normalized: true,
        query_prefix: String::new(),
        passage_prefix: String::new(),
        tokenizer: "pkms-estimated-tokens-v1".into(),
        chunker: "pkms-structure-first-v2:block-line-sentence:overlap".into(),
        max_tokens: tokens,
        overlap_tokens: overlap,
        provider: None,
        output_dimensions: None,
    }
}

#[test]
fn chunk_settings_default_roundtrip_replay_and_conflict() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let initial = library.chunk_settings().unwrap();
    assert_eq!(initial.params, ChunkParams::default());
    assert_eq!(initial.params.chunk_tokens, 512);
    assert_eq!(initial.params.overlap_percent, 15);
    assert_eq!(initial.row_version, 0);

    let command = id();
    let request = SetChunkParams {
        base_row_version: 0,
        params: ChunkParams {
            chunk_tokens: 256,
            overlap_percent: 10,
        },
    };
    let saved = library.set_chunk_params(&command, request).unwrap();
    assert_eq!(saved.settings.row_version, 1);
    assert!(!saved.receipt.replayed);
    assert_eq!(library.chunk_settings().unwrap(), saved.settings);
    let replayed = library.set_chunk_params(&command, request).unwrap();
    assert!(replayed.receipt.replayed);
    assert_eq!(replayed.settings, saved.settings);
    assert!(matches!(
        library.set_chunk_params(
            &id(),
            SetChunkParams {
                base_row_version: 0,
                params: ChunkParams::default(),
            }
        ),
        Err(Error::MetadataConflict { .. })
    ));
    for params in [
        ChunkParams {
            chunk_tokens: 32,
            overlap_percent: 15,
        },
        ChunkParams {
            chunk_tokens: 2048,
            overlap_percent: 15,
        },
        ChunkParams {
            chunk_tokens: 512,
            overlap_percent: 40,
        },
    ] {
        assert!(matches!(
            library.set_chunk_params(
                &id(),
                SetChunkParams {
                    base_row_version: 1,
                    params,
                }
            ),
            Err(Error::InvalidInput(_))
        ));
    }
    assert_eq!(library.chunk_settings().unwrap(), saved.settings);
    drop(library);
    let reopened = Library::open_existing(root.path()).unwrap();
    assert_eq!(reopened.chunk_settings().unwrap(), saved.settings);
}

#[test]
fn switching_provider_or_model_registers_a_new_profile_and_requeues_objects() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library
        .create_note("换模型验证", "正文内容用于标脏")
        .unwrap();
    let cloudflare = library
        .register_embedding_profile(&id(), spec(512, 76))
        .unwrap()
        .profile;
    // Re-saving the same configuration keeps the identity, so nothing rebuilds.
    let again = library
        .register_embedding_profile(&id(), cloudflare.spec.clone())
        .unwrap()
        .profile;
    assert_eq!(again.id, cloudflare.id);

    let mut openai = spec(512, 76);
    openai.model = "text-embedding-3-small".into();
    openai.revision = "openai:text-embedding-3-small:v1".into();
    openai.dimensions = 1536;
    openai.provider = Some(EmbeddingProvider::Openai);
    let switched = library
        .register_embedding_profile(&id(), openai)
        .unwrap()
        .profile;
    assert_ne!(
        switched.id, cloudflare.id,
        "the provider and model are part of the profile identity"
    );
    assert_eq!(
        library.previous_embedding_profile().unwrap().unwrap().id,
        cloudflare.id
    );
    assert!(
        library
            .dirty_index_object_ids(32)
            .unwrap()
            .contains(&note.id),
        "a new model must rebuild the index instead of mixing vector spaces"
    );
}

#[test]
fn a_new_chunk_cap_swaps_the_profile_and_can_be_rolled_back() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    let note = library.create_note("回退验证", "正文内容用于标脏").unwrap();
    assert!(library.previous_embedding_profile().unwrap().is_none());

    let first = library
        .register_embedding_profile(&id(), spec(512, 76))
        .unwrap()
        .profile;
    assert!(library.previous_embedding_profile().unwrap().is_none());

    let saved = library
        .set_chunk_params(
            &id(),
            SetChunkParams {
                base_row_version: 0,
                params: ChunkParams {
                    chunk_tokens: 256,
                    overlap_percent: 10,
                },
            },
        )
        .unwrap();
    assert_eq!(saved.settings.params.chunk_tokens, 256);

    let second = library
        .register_embedding_profile(&id(), spec(256, 25))
        .unwrap()
        .profile;
    assert_ne!(
        first.id, second.id,
        "the chunk cap is part of the profile identity"
    );
    assert_eq!(
        library.active_embedding_profile().unwrap().unwrap().id,
        second.id
    );
    assert_eq!(
        library.previous_embedding_profile().unwrap().unwrap().id,
        first.id
    );

    // Rolling back re-activates the previous profile and re-queues the objects so
    // the existing generations are reused instead of being embedded again.
    let reverted = library.revert_embedding_profile(&id()).unwrap();
    assert_eq!(reverted.profile.id, first.id);
    assert_eq!(
        library.active_embedding_profile().unwrap().unwrap().id,
        first.id
    );
    assert_eq!(
        library.previous_embedding_profile().unwrap().unwrap().id,
        second.id
    );
    assert!(
        library
            .dirty_index_object_ids(32)
            .unwrap()
            .contains(&note.id)
    );

    // Reverting again goes forward, and a replayed command returns the same result.
    let command = id();
    let forward = library.revert_embedding_profile(&command).unwrap();
    assert_eq!(forward.profile.id, second.id);
    let replay = library.revert_embedding_profile(&command).unwrap();
    assert!(replay.receipt.replayed);
    assert_eq!(replay.profile.id, second.id);
    assert_eq!(
        library.active_embedding_profile().unwrap().unwrap().id,
        second.id
    );
}

#[test]
fn reverting_without_a_previous_profile_is_rejected() {
    let root = tempdir().unwrap();
    let mut library = Library::open(root.path()).unwrap();
    assert!(matches!(
        library.revert_embedding_profile(&id()),
        Err(Error::InvalidInput(_))
    ));
}

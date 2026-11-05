//! Immutable candidate profile; available without loading any model runtime.
use pkms_core::EmbeddingProfileSpec;
pub const MODEL_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";
pub const MODEL_HASH: &str = "ca456c06b3a9505ddfd9131408916dd79290368331e7d76bb621f1cba6bc8665";
pub const TOKENIZER_HASH: &str = "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39";
pub const DIMENSIONS: usize = 384;
pub const MAX_TOKENS: usize = 512;
/// Conservative text and managed-template admission, not an exact tokenizer count.
pub fn chat_context_bound(
    content_utf8_bytes: usize,
    message_count: usize,
    num_predict: u32,
) -> usize {
    content_utf8_bytes
        .saturating_add(256)
        .saturating_add(message_count.saturating_mul(64))
        .saturating_add(num_predict as usize)
}
pub fn profile_spec() -> EmbeddingProfileSpec {
    EmbeddingProfileSpec {
        endpoint: None,
        model: "intfloat/multilingual-e5-small/onnx/model.onnx".into(),
        revision: format!("{MODEL_REVISION}:sha256:{MODEL_HASH}"),
        dimensions: DIMENSIONS as u32,
        normalized: true,
        query_prefix: "query: ".into(),
        passage_prefix: "passage: ".into(),
        tokenizer: format!("tokenizers-0.23.2:sha256:{TOKENIZER_HASH}"),
        chunker: "pkms-token-boundary-v1:title-newline:no-overlap".into(),
        max_tokens: MAX_TOKENS as u32,
        overlap_tokens: 0,
        provider: None,
        output_dimensions: None,
    }
}

//! Fixed claims-v1 output grammar. Callers can select evidence IDs, not a schema.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatFormat {
    EvidenceAnswer { citation_ids: Vec<String> },
}

#[derive(Debug, thiserror::Error)]
#[error("model_invalid_request")]
pub struct InvalidChatFormat;

impl ChatFormat {
    /// This schema intentionally matches the independently probed
    /// scripts/w4-quality/claims_protocol.py. The App must additionally validate
    /// semantic bindings and forbidden citation markup before committing text.
    pub fn json_schema(&self) -> Result<Value, InvalidChatFormat> {
        let Self::EvidenceAnswer { citation_ids } = self;
        if !(1..=32).contains(&citation_ids.len()) {
            return Err(InvalidChatFormat);
        }
        let mut seen = [false; 32];
        for id in citation_ids {
            if !(2..=3).contains(&id.len()) {
                return Err(InvalidChatFormat);
            }
            let Some(number) = id.strip_prefix('E').and_then(|s| s.parse::<usize>().ok()) else {
                return Err(InvalidChatFormat);
            };
            if !(1..=32).contains(&number) || *id != format!("E{number}") || seen[number - 1] {
                return Err(InvalidChatFormat);
            }
            seen[number - 1] = true;
        }
        let claim = json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "minLength": 1, "maxLength": 1024, "pattern": "^[^\r\n]+$"},
                "citation_ids": {
                    "type": "array", "items": {"type": "string", "enum": citation_ids},
                    "minItems": 1, "maxItems": citation_ids.len(), "uniqueItems": true
                }
            },
            "required": ["text", "citation_ids"], "additionalProperties": false
        });
        Ok(json!({"anyOf": [
            {
                "type": "object",
                "properties": {
                    "status": {"const": "insufficient_evidence"},
                    "claims": {"type": "array", "items": claim, "maxItems": 0}
                },
                "required": ["status", "claims"], "additionalProperties": false
            },
            {
                "type": "object",
                "properties": {
                    "status": {"const": "answered"},
                    "claims": {"type": "array", "items": claim, "minItems": 1, "maxItems": 16}
                },
                "required": ["status", "claims"], "additionalProperties": false
            }
        ]}))
    }
}

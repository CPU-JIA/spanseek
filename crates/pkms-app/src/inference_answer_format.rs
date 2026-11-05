//! Decode the constrained claims protocol before any model text reaches answer storage.
use crate::{AppError, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Answered,
    InsufficientEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    status: Status,
    claims: Vec<Claim>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    text: String,
    citation_ids: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Answer {
    InsufficientEvidence,
    Supported {
        text: String,
        citations: Vec<String>,
    },
}

fn invalid() -> AppError {
    AppError::InvalidInput("model_refusal_protocol_invalid")
}

fn citation_markup(text: &str) -> bool {
    text.split('[').skip(1).any(|tail| {
        let Some((inside, _)) = tail.split_once(']') else {
            return false;
        };
        let candidate = inside
            .rsplit_once([':', '：'])
            .map_or(inside, |(_, suffix)| suffix.trim_start());
        candidate.strip_prefix('E').is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

pub(crate) fn decode(raw: &str, allowed: &[String]) -> Result<Answer> {
    // Derived struct decoding rejects duplicate keys at both nesting levels.
    let response: Response = serde_json::from_str(raw).map_err(|_| invalid())?;
    if response.status == Status::InsufficientEvidence {
        return if response.claims.is_empty() {
            Ok(Answer::InsufficientEvidence)
        } else {
            Err(invalid())
        };
    }
    if response.claims.is_empty() || response.claims.len() > 16 {
        return Err(invalid());
    }
    let mut citations = Vec::new();
    let mut lines = Vec::new();
    for claim in response.claims {
        if claim.text.trim().is_empty()
            || claim.text.chars().count() > 1024
            || claim.text.contains(['\r', '\n'])
            || claim.text.contains(pkms_core::INSUFFICIENT_EVIDENCE_MARKER)
            || citation_markup(&claim.text)
            || claim.citation_ids.is_empty()
            || claim.citation_ids.len() > allowed.len()
        {
            return Err(invalid());
        }
        let mut line = format!("- {} ", claim.text);
        for (index, id) in claim.citation_ids.iter().enumerate() {
            if !allowed.contains(id) || claim.citation_ids[..index].contains(id) {
                return Err(invalid());
            }
            if !citations.contains(id) {
                citations.push(id.clone());
            }
            line.push('[');
            line.push_str(id);
            line.push(']');
        }
        lines.push(line);
    }
    Ok(Answer::Supported {
        text: lines.join("\n"),
        citations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(raw: &str) -> Result<Answer> {
        decode(raw, &["E1".into(), "E2".into()])
    }

    #[test]
    fn each_claim_keeps_its_own_citations_and_wire_json_is_never_rendered() {
        assert_eq!(parse(r#"{"status":"answered","claims":[{"text":"甲有6项。","citation_ids":["E1"]},{"text":"乙有3项。","citation_ids":["E2","E1"]}]}"#).unwrap(), Answer::Supported {
            text: "- 甲有6项。 [E1]\n- 乙有3项。 [E2][E1]".into(),
            citations: vec!["E1".into(), "E2".into()],
        });
        assert_eq!(
            parse(r#"{"status":"insufficient_evidence","claims":[]}"#).unwrap(),
            Answer::InsufficientEvidence
        );
    }

    #[test]
    fn invalid_structure_refusals_or_claim_bindings_are_rejected() {
        for raw in [
            r#"{"status":"answered","status":"insufficient_evidence","claims":[]}"#,
            r#"{"status":"insufficient_evidence","claims":[],"extra":true}"#,
            r#"{"status":"answered","claims":[]}"#,
            r#"{"status":"insufficient_evidence","claims":[{"text":"claim","citation_ids":["E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"claim","text":"replacement","citation_ids":["E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"claim","citation_ids":["E3"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"claim","citation_ids":["E1","E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"claim","citation_ids":[] }]}"#,
            r#"{"status":"answered","claims":[{"text":"claim[E1]","citation_ids":["E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"claim[依据：E1]","citation_ids":["E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"[[PKMS_INSUFFICIENT_EVIDENCE]]","citation_ids":["E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":"claim\nsecond","citation_ids":["E1"]}]}"#,
            r#"{"status":"answered","claims":[{"text":null,"citation_ids":["E1"]}]}"#,
        ] {
            assert!(parse(raw).is_err(), "accepted invalid result: {raw}");
        }
    }

    #[test]
    fn unicode_and_claim_count_limits_match_the_schema() {
        let claim = |text| serde_json::json!({"text": text, "citation_ids": ["E1"]});
        let response =
            |claims| serde_json::json!({"status":"answered", "claims": claims}).to_string();
        assert!(parse(&response(vec![claim("中".repeat(1024))])).is_ok());
        assert!(parse(&response(vec![claim("中".repeat(1025))])).is_err());
        assert!(parse(&response(vec![claim("事实".into()); 16])).is_ok());
        assert!(parse(&response(vec![claim("事实".into()); 17])).is_err());
    }
}

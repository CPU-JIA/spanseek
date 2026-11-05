//! Stable external command contract shared by the MCP and local HTTP surfaces.
//!
//! This module deliberately contains no transport, database, or network code.
//! Both adapters decode the same envelope, apply the same validation, and map
//! failures to the same error codes. A breaking change requires a new protocol
//! identifier (`spanseek.external/v2`); additive optional fields stay in v1.
use crate::{NoteDocument, SearchMode, SearchScope};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EXTERNAL_PROTOCOL: &str = "spanseek.external/v1";
pub const EXTERNAL_PROTOCOL_VERSION: u32 = 1;
pub const EXTERNAL_MAX_REQUEST_ID: usize = 128;
pub const EXTERNAL_MAX_PAGE_SIZE: u32 = 100;

/// A permission granted to one external client. The transport decides how the
/// grant is obtained (stdio is local-only; HTTP uses a bearer token), while the
/// dispatcher applies the same scope checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalScope {
    Read,
    Search,
    Propose,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalErrorCode {
    UnsupportedProtocol,
    InvalidRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unavailable,
    WriteProposalOnly,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalError {
    pub code: ExternalErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_scope: Option<ExternalScope>,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRequest {
    pub protocol: String,
    pub request_id: String,
    pub command: ExternalCommand,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExternalCommand {
    Capabilities,
    Search(ExternalSearchParams),
    ReadNote(ExternalReadNoteParams),
    ListNotes(ExternalListNotesParams),
    ProposeNote(ExternalProposeNoteParams),
    AuditPage(ExternalAuditPageParams),
}

impl ExternalCommand {
    pub fn required_scope(&self) -> ExternalScope {
        match self {
            Self::Capabilities | Self::ReadNote(_) | Self::ListNotes(_) => ExternalScope::Read,
            Self::Search(_) => ExternalScope::Search,
            Self::ProposeNote(_) => ExternalScope::Propose,
            Self::AuditPage(_) => ExternalScope::Audit,
        }
    }

    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Search(_) => "search",
            Self::ReadNote(_) => "read_note",
            Self::ListNotes(_) => "list_notes",
            Self::ProposeNote(_) => "propose_note",
            Self::AuditPage(_) => "audit_page",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalSearchParams {
    pub query: String,
    pub mode: SearchMode,
    #[serde(default)]
    pub scope: SearchScope,
    pub limit: u32,
    #[serde(default)]
    pub minimum_cosine: Option<f64>,
}

impl From<ExternalSearchParams> for crate::SearchQuery {
    fn from(value: ExternalSearchParams) -> Self {
        Self {
            query: value.query,
            mode: value.mode,
            scope: value.scope,
            limit: value.limit,
            minimum_cosine: value.minimum_cosine,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalReadNoteParams {
    pub note_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalListNotesParams {
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default)]
    pub trashed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalProposeNoteParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision_id: Option<String>,
    pub title: String,
    pub document_schema: u32,
    pub document: NoteDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuditPageParams {
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAuditOutcome {
    Allowed,
    Denied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuditDraft {
    pub request_id: String,
    pub transport: String,
    pub principal_id: String,
    pub method: String,
    pub required_scope: ExternalScope,
    pub granted_scopes: Vec<ExternalScope>,
    pub outcome: ExternalAuditOutcome,
    pub error_code: Option<ExternalErrorCode>,
    pub subject_id: Option<String>,
    pub duration_ms: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuditEvent {
    pub seq: i64,
    pub request_id: String,
    pub occurred_at: String,
    pub transport: String,
    pub principal_id: String,
    pub method: String,
    pub required_scope: ExternalScope,
    pub granted_scopes: Vec<ExternalScope>,
    pub outcome: ExternalAuditOutcome,
    pub error_code: Option<ExternalErrorCode>,
    pub subject_id: Option<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuditPage {
    pub items: Vec<ExternalAuditEvent>,
    pub next_before_seq: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalResponse {
    pub protocol: String,
    pub request_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ExternalError>,
}

impl ExternalResponse {
    pub fn success(request_id: impl Into<String>, data: Value) -> Self {
        Self {
            protocol: EXTERNAL_PROTOCOL.into(),
            request_id: request_id.into(),
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(request_id: impl Into<String>, error: ExternalError) -> Self {
        Self {
            protocol: EXTERNAL_PROTOCOL.into(),
            request_id: request_id.into(),
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalCapabilities {
    pub protocol: String,
    pub protocol_version: u32,
    pub supported_protocols: Vec<String>,
    pub read_only_default: bool,
    pub write_mode: String,
    pub methods: Vec<String>,
    pub scopes: Vec<ExternalScope>,
    pub max_page_size: u32,
    pub compatibility: String,
}

impl ExternalCapabilities {
    pub fn v1() -> Self {
        Self {
            protocol: EXTERNAL_PROTOCOL.into(),
            protocol_version: EXTERNAL_PROTOCOL_VERSION,
            supported_protocols: vec![EXTERNAL_PROTOCOL.into()],
            read_only_default: true,
            write_mode: "proposal_only".into(),
            methods: vec![
                "capabilities".into(),
                "search".into(),
                "read_note".into(),
                "list_notes".into(),
                "propose_note".into(),
                "audit_page".into(),
            ],
            scopes: vec![
                ExternalScope::Read,
                ExternalScope::Search,
                ExternalScope::Propose,
                ExternalScope::Audit,
            ],
            max_page_size: EXTERNAL_MAX_PAGE_SIZE,
            compatibility: "Additive optional fields remain in v1; breaking changes use a new protocol version.".into(),
        }
    }
}

impl ExternalRequest {
    pub fn decode(json: &str) -> std::result::Result<Self, ExternalError> {
        let value: Value =
            serde_json::from_str(json).map_err(|_| invalid_request("request is not valid JSON"))?;
        let protocol = value
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let request_id = value
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if protocol != EXTERNAL_PROTOCOL {
            return Err(ExternalError {
                code: ExternalErrorCode::UnsupportedProtocol,
                message: format!("expected {EXTERNAL_PROTOCOL}"),
                required_scope: None,
                retryable: false,
            });
        }
        let request: Self = serde_json::from_value(value)
            .map_err(|_| invalid_request("request does not match the v1 schema"))?;
        request.validate().map_err(|message| ExternalError {
            code: ExternalErrorCode::InvalidRequest,
            message: message.into(),
            required_scope: None,
            retryable: false,
        })?;
        Ok(Self {
            request_id,
            ..request
        })
    }

    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.protocol != EXTERNAL_PROTOCOL {
            return Err("unsupported external protocol");
        }
        if self.request_id.is_empty()
            || self.request_id.len() > EXTERNAL_MAX_REQUEST_ID
            || self.request_id.chars().any(char::is_control)
        {
            return Err("request_id must contain 1-128 non-control characters");
        }
        match &self.command {
            ExternalCommand::Search(params) => {
                if params.query.len() > 4096
                    || !(1..=EXTERNAL_MAX_PAGE_SIZE).contains(&params.limit)
                {
                    return Err("query or limit exceeds bounds");
                }
                crate::SearchQuery::from(params.clone())
                    .validate()
                    .map_err(|_| "invalid search request")?;
            }
            ExternalCommand::ListNotes(params) => {
                if !(1..=EXTERNAL_MAX_PAGE_SIZE).contains(&params.limit) {
                    return Err("limit exceeds bounds");
                }
                if params.query.as_ref().is_some_and(|query| query.len() > 512) {
                    return Err("title query exceeds bounds");
                }
            }
            ExternalCommand::AuditPage(params) => {
                if !(1..=EXTERNAL_MAX_PAGE_SIZE).contains(&params.limit)
                    || params
                        .request_id
                        .as_ref()
                        .is_some_and(|value| value.len() > EXTERNAL_MAX_REQUEST_ID)
                    || params
                        .principal_id
                        .as_ref()
                        .is_some_and(|value| value.is_empty() || value.len() > 128)
                    || params
                        .subject_id
                        .as_ref()
                        .is_some_and(|value| value.len() > 128)
                {
                    return Err("audit filter exceeds bounds");
                }
            }
            ExternalCommand::ReadNote(params) => {
                crate::document::valid_uuid(&params.note_id)
                    .map_err(|_| "note_id must be a UUID")?;
                if let Some(revision) = &params.revision_id {
                    crate::document::valid_uuid(revision)
                        .map_err(|_| "revision_id must be a UUID")?;
                }
            }
            ExternalCommand::ProposeNote(params) => {
                if params.title.trim().is_empty() || params.title.chars().count() > 300 {
                    return Err("title must contain 1-300 characters");
                }
                if params.note_id.is_some() != params.base_revision_id.is_some() {
                    return Err("note_id and base_revision_id must both be present or absent");
                }
            }
            ExternalCommand::Capabilities => {}
        }
        Ok(())
    }
}

fn invalid_request(message: &'static str) -> ExternalError {
    ExternalError {
        code: ExternalErrorCode::InvalidRequest,
        message: message.into(),
        required_scope: None,
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_request() -> ExternalRequest {
        ExternalRequest {
            protocol: EXTERNAL_PROTOCOL.into(),
            request_id: "req-1".into(),
            command: ExternalCommand::Search(ExternalSearchParams {
                query: "知识".into(),
                mode: SearchMode::Lexical,
                scope: SearchScope::default(),
                limit: 10,
                minimum_cosine: None,
            }),
        }
    }

    #[test]
    fn v1_request_round_trips_with_a_stable_envelope() {
        let request = search_request();
        let json = serde_json::to_string(&request).unwrap();
        let decoded = ExternalRequest::decode(&json).unwrap();
        assert_eq!(decoded, request);
        assert_eq!(decoded.command.required_scope(), ExternalScope::Search);
    }

    #[test]
    fn unknown_protocol_is_rejected_before_command_dispatch() {
        let mut value = serde_json::to_value(search_request()).unwrap();
        value["protocol"] = Value::String("spanseek.external/v2".into());
        let error = ExternalRequest::decode(&value.to_string()).unwrap_err();
        assert_eq!(error.code, ExternalErrorCode::UnsupportedProtocol);
    }

    #[test]
    fn unknown_command_and_parameter_fields_are_rejected() {
        let command = r#"{"protocol":"spanseek.external/v1","request_id":"r","command":{"method":"unknown","params":{}}}"#;
        assert_eq!(
            ExternalRequest::decode(command).unwrap_err().code,
            ExternalErrorCode::InvalidRequest
        );
        let field = r#"{"protocol":"spanseek.external/v1","request_id":"r","command":{"method":"list_notes","params":{"limit":1,"unexpected":true}}}"#;
        assert_eq!(
            ExternalRequest::decode(field).unwrap_err().code,
            ExternalErrorCode::InvalidRequest
        );
    }

    #[test]
    fn request_ids_and_page_sizes_are_bounded() {
        let mut request = search_request();
        request.request_id = "x".repeat(EXTERNAL_MAX_REQUEST_ID + 1);
        assert!(request.validate().is_err());
        request.request_id = "ok".into();
        if let ExternalCommand::Search(params) = &mut request.command {
            params.limit = EXTERNAL_MAX_PAGE_SIZE + 1;
        }
        assert!(request.validate().is_err());
    }

    #[test]
    fn capabilities_are_read_only_by_default_and_name_the_protocol() {
        let capabilities = ExternalCapabilities::v1();
        assert_eq!(capabilities.protocol, EXTERNAL_PROTOCOL);
        assert!(capabilities.read_only_default);
        assert_eq!(capabilities.write_mode, "proposal_only");
        assert_eq!(
            capabilities.supported_protocols,
            vec![EXTERNAL_PROTOCOL.to_owned()]
        );
    }
}

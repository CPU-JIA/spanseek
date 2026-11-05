use crate::service::{ExternalGrant, ExternalService};
use pkms_core::{
    ExternalCommand, ExternalListNotesParams, ExternalProposeNoteParams, ExternalReadNoteParams,
    ExternalRequest, ExternalScope, ExternalSearchParams,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    io::{BufRead, Write},
};

pub struct McpSession {
    service: ExternalService,
}

impl McpSession {
    pub fn new(service: ExternalService) -> Self {
        Self { service }
    }

    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = message.get("id").cloned();
        if id.is_none() && method != "notifications/initialized" {
            return None;
        }
        match method {
            "initialize" => rpc_success(id, initialize_result()),
            "notifications/initialized" => None,
            "ping" => rpc_success(id, json!({})),
            "tools/list" => rpc_success(id, json!({ "tools": tools(self.service.policy()) })),
            "tools/call" => {
                self.call_tool(id, message.get("params").cloned().unwrap_or(Value::Null))
            }
            _ => Some(rpc_error(id, -32601, "method not found")),
        }
    }

    fn call_tool(&mut self, id: Option<Value>, params: Value) -> Option<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Err(message) = validate_arguments(name, &arguments) {
            return Some(tool_error(id, &message));
        }
        let (command, scopes) = match tool_request(name, &arguments, self.service.policy()) {
            Ok(value) => value,
            Err(message) => {
                return Some(tool_error(id, &message));
            }
        };
        let request = ExternalRequest {
            protocol: pkms_core::EXTERNAL_PROTOCOL.into(),
            request_id: format!("mcp-{}", uuid::Uuid::new_v4()),
            command,
        };
        let response = self
            .service
            .dispatch_as(&request, &ExternalGrant::mcp(scopes));
        Some(tool_result(id, &response))
    }
}

pub fn serve_stdio(mut session: McpSession) -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => session.handle(message),
            Err(_) => Some(rpc_error(None, -32700, "parse error")),
        };
        if let Some(response) = response {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": { "name": "spanseek", "version": "1.9.6" },
        "instructions": "spanseek is read-only by default. Search and read return evidence; writes only create proposals when explicitly enabled."
    })
}

fn tools(policy: crate::service::ServicePolicy) -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "spanseek_capabilities",
            "description": "Show the spanseek external protocol, methods, scopes and write policy.",
            "inputSchema": { "type": "object", "additionalProperties": false, "properties": {} }
        }),
        json!({
            "name": "spanseek_search",
            "description": "Search private notes and sources. Returns evidence with stable targets; it never writes.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": { "type": "string", "maxLength": 4096 },
                    "mode": { "type": "string", "enum": ["lexical", "semantic", "hybrid"], "default": "lexical" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10 },
                    "scope": { "type": "object" },
                    "minimum_cosine": { "type": ["number", "null"] }
                }
            }
        }),
        json!({
            "name": "spanseek_read_note",
            "description": "Read one note revision, or the current revision when revision_id is omitted.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["note_id"],
                "properties": {
                    "note_id": { "type": "string" },
                    "revision_id": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "spanseek_list_notes",
            "description": "List bounded note summaries without returning full document bodies.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
                    "cursor": { "type": "string" },
                    "query": { "type": "string", "maxLength": 512 },
                    "trashed": { "type": "boolean", "default": false }
                }
            }
        }),
    ];
    if policy.allow_proposals {
        tools.push(json!({
            "name": "spanseek_propose_note",
            "description": "Create a pending proposal for human review. This never applies or overwrites the note.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["title", "document"],
                "properties": {
                    "note_id": { "type": "string" },
                    "base_revision_id": { "type": "string" },
                    "title": { "type": "string" },
                    "document_schema": { "type": "integer", "default": pkms_core::DOCUMENT_SCHEMA },
                    "document": { "type": "object" }
                }
            }
        }));
    }
    if policy.allow_audit {
        tools.push(json!({
            "name": "spanseek_audit",
            "description": "Read recent external/automation audit events. It never changes state.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
                    "before_seq": { "type": "integer" },
                    "request_id": { "type": "string", "maxLength": 128 },
                    "principal_id": { "type": "string", "maxLength": 128 },
                    "subject_id": { "type": "string", "maxLength": 128 }
                }
            }
        }));
    }
    tools
}

fn tool_request(
    name: &str,
    arguments: &Value,
    policy: crate::service::ServicePolicy,
) -> Result<(ExternalCommand, BTreeSet<ExternalScope>), String> {
    let mut scopes = BTreeSet::from([ExternalScope::Read, ExternalScope::Search]);
    let command = match name {
        "spanseek_capabilities" => ExternalCommand::Capabilities,
        "spanseek_search" => {
            let params: ExternalSearchParams = serde_json::from_value(json!({
                "query": required_string(arguments, "query")?,
                "mode": arguments.get("mode").cloned().unwrap_or_else(|| json!("lexical")),
                "scope": arguments.get("scope").cloned().unwrap_or_else(|| json!({})),
                "limit": arguments.get("limit").and_then(Value::as_u64).unwrap_or(10),
                "minimum_cosine": arguments.get("minimum_cosine").cloned().unwrap_or(Value::Null),
            }))
            .map_err(|_| "invalid search arguments".to_owned())?;
            ExternalCommand::Search(params)
        }
        "spanseek_read_note" => ExternalCommand::ReadNote(ExternalReadNoteParams {
            note_id: required_string(arguments, "note_id")?,
            revision_id: optional_string(arguments, "revision_id")?,
        }),
        "spanseek_list_notes" => ExternalCommand::ListNotes(ExternalListNotesParams {
            limit: arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as u32,
            cursor: optional_string(arguments, "cursor")?,
            query: optional_string(arguments, "query")?,
            trashed: arguments
                .get("trashed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        "spanseek_propose_note" if policy.allow_proposals => {
            scopes.insert(ExternalScope::Propose);
            let document = arguments
                .get("document")
                .cloned()
                .ok_or_else(|| "document is required".to_owned())?;
            let document = serde_json::from_value(document)
                .map_err(|_| "document does not match the note document schema".to_owned())?;
            ExternalCommand::ProposeNote(ExternalProposeNoteParams {
                note_id: optional_string(arguments, "note_id")?,
                base_revision_id: optional_string(arguments, "base_revision_id")?,
                title: required_string(arguments, "title")?,
                document_schema: arguments
                    .get("document_schema")
                    .and_then(Value::as_u64)
                    .unwrap_or(pkms_core::DOCUMENT_SCHEMA as u64)
                    as u32,
                document,
            })
        }
        "spanseek_audit" if policy.allow_audit => {
            scopes.insert(ExternalScope::Audit);
            ExternalCommand::AuditPage(pkms_core::ExternalAuditPageParams {
                limit: arguments.get("limit").and_then(Value::as_u64).unwrap_or(20) as u32,
                before_seq: arguments.get("before_seq").and_then(Value::as_i64),
                request_id: optional_string(arguments, "request_id")?,
                principal_id: optional_string(arguments, "principal_id")?,
                subject_id: optional_string(arguments, "subject_id")?,
            })
        }
        _ => return Err("tool is not enabled for this server".into()),
    };
    Ok((command, scopes))
}

fn validate_arguments(name: &str, arguments: &Value) -> Result<(), String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "tool arguments must be an object".to_owned())?;
    let allowed: &[&str] = match name {
        "spanseek_capabilities" => &[],
        "spanseek_search" => &["query", "mode", "limit", "scope", "minimum_cosine"],
        "spanseek_read_note" => &["note_id", "revision_id"],
        "spanseek_list_notes" => &["limit", "cursor", "query", "trashed"],
        "spanseek_propose_note" => &[
            "note_id",
            "base_revision_id",
            "title",
            "document_schema",
            "document",
        ],
        "spanseek_audit" => &[
            "limit",
            "before_seq",
            "request_id",
            "principal_id",
            "subject_id",
        ],
        _ => return Ok(()),
    };
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unknown argument: {key}"));
    }
    Ok(())
}

fn required_string(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{key} is required"))
}

fn optional_string(arguments: &Value, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn tool_result(id: Option<Value>, response: &pkms_core::ExternalResponse) -> Value {
    let text = serde_json::to_string(response).unwrap_or_else(|_| "{\"ok\":false}".into());
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": text }],
            "isError": !response.ok
        }
    })
}

fn tool_error(id: Option<Value>, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }
    })
}

fn rpc_success(id: Option<Value>, result: Value) -> Option<Value> {
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn rpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ExternalService, ServicePolicy};
    use pkms_core::Library;

    fn session() -> (tempfile::TempDir, McpSession, String) {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::open(directory.path()).unwrap();
        let note = library.create_note("MCP 测试", "外部检索证据").unwrap();
        let service = ExternalService::new(library, ServicePolicy::default());
        (directory, McpSession::new(service), note.id)
    }

    #[test]
    fn initialize_and_list_tools_default_to_read_only() {
        let (_directory, mut session, _note) = session();
        let init = session
            .handle(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}))
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "spanseek");
        let list = session
            .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
            .unwrap();
        let names = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"spanseek_search"));
        assert!(!names.contains(&"spanseek_propose_note"));
    }

    #[test]
    fn read_only_mcp_search_returns_evidence_without_changing_the_note() {
        let (_directory, mut session, note_id) = session();
        let before = session.service.library().note(&note_id).unwrap();
        let response = session
            .handle(json!({
                "jsonrpc":"2.0",
                "id":3,
                "method":"tools/call",
                "params":{"name":"spanseek_search","arguments":{"query":"外部"}}
            }))
            .unwrap();
        assert_eq!(response["result"]["isError"], false);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let envelope: pkms_core::ExternalResponse = serde_json::from_str(text).unwrap();
        assert!(envelope.ok);
        let after = session.service.library().note(&note_id).unwrap();
        assert_eq!(before.revision_id, after.revision_id);
    }

    fn session_with_policy(policy: ServicePolicy) -> (tempfile::TempDir, McpSession, String) {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::open(directory.path()).unwrap();
        let note = library
            .create_note("MCP 提案测试", "外部检索与提案证据")
            .unwrap();
        let service = ExternalService::new(library, policy);
        (directory, McpSession::new(service), note.id)
    }

    fn envelope(response: &Value) -> pkms_core::ExternalResponse {
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn read_search_and_list_leave_the_note_revision_unchanged() {
        let (_directory, mut session, note_id) = session();
        let before = session.service.library().note(&note_id).unwrap();
        for (id, name, arguments) in [
            (10, "spanseek_search", json!({"query":"外部"})),
            (11, "spanseek_read_note", json!({"note_id":note_id})),
            (12, "spanseek_list_notes", json!({"limit":5})),
            (13, "spanseek_capabilities", json!({})),
        ] {
            let response = session
                .handle(json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "method":"tools/call",
                    "params":{"name":name,"arguments":arguments}
                }))
                .unwrap();
            assert_eq!(response["result"]["isError"], false, "{response}");
            assert!(envelope(&response).ok);
        }
        let after = session.service.library().note(&note_id).unwrap();
        assert_eq!(before.revision_id, after.revision_id);
        assert_eq!(before.content_hash, after.content_hash);
    }

    #[test]
    fn disabled_proposal_tool_is_not_callable() {
        let (_directory, mut session, note_id) = session();
        let response = session
            .handle(json!({
                "jsonrpc":"2.0",
                "id":20,
                "method":"tools/call",
                "params":{
                    "name":"spanseek_propose_note",
                    "arguments":{"title":"外部提案","document":{"type":"doc","content":[]}}
                }
            }))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        let after = session.service.library().note(&note_id).unwrap();
        assert_eq!(after.revision, 1);
    }

    #[test]
    fn enabled_proposal_creates_only_a_pending_review_record() {
        let (_directory, mut session, note_id) = session_with_policy(ServicePolicy {
            allow_proposals: true,
            allow_audit: false,
        });
        let before = session.service.library().note(&note_id).unwrap();
        let response = session
            .handle(json!({
                "jsonrpc":"2.0",
                "id":21,
                "method":"tools/call",
                "params":{
                    "name":"spanseek_propose_note",
                    "arguments":{
                        "note_id":note_id,
                        "base_revision_id":before.revision_id,
                        "title":"外部提案",
                        "document":before.document
                    }
                }
            }))
            .unwrap();
        assert_eq!(response["result"]["isError"], false, "{response}");
        let envelope = envelope(&response);
        assert!(envelope.ok);
        assert_eq!(envelope.data.unwrap()["status"], "pending");
        let after = session.service.library().note(&note_id).unwrap();
        assert_eq!(before.revision_id, after.revision_id);
        assert_eq!(before.content_hash, after.content_hash);
    }

    #[test]
    fn unknown_tool_arguments_are_rejected_before_dispatch() {
        let (_directory, mut session, _note) = session();
        let response = session
            .handle(json!({
                "jsonrpc":"2.0",
                "id":22,
                "method":"tools/call",
                "params":{"name":"spanseek_search","arguments":{"query":"外部","body":"ignored"}}
            }))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown argument: body")
        );
    }
}

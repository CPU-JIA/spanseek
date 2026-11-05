use pkms_core::{
    CreateProposal, ExternalAuditDraft, ExternalAuditOutcome, ExternalAuditPageParams,
    ExternalCapabilities, ExternalCommand, ExternalError, ExternalErrorCode,
    ExternalListNotesParams, ExternalProposeNoteParams, ExternalReadNoteParams, ExternalRequest,
    ExternalResponse, ExternalScope, ExternalSearchParams, Library, NotesPageRequest, SearchMode,
};
use std::{collections::BTreeSet, time::Instant};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServicePolicy {
    pub allow_proposals: bool,
    pub allow_audit: bool,
}

#[derive(Debug, Clone)]
pub struct ExternalGrant {
    pub principal_id: String,
    pub transport: String,
    pub scopes: BTreeSet<ExternalScope>,
}

impl ExternalGrant {
    pub fn mcp(scopes: BTreeSet<ExternalScope>) -> Self {
        Self {
            principal_id: "mcp-local".into(),
            transport: "mcp".into(),
            scopes,
        }
    }

    pub fn http(principal_id: impl Into<String>, scopes: BTreeSet<ExternalScope>) -> Self {
        Self {
            principal_id: principal_id.into(),
            transport: "http".into(),
            scopes,
        }
    }
}

pub struct ExternalService {
    library: Library,
    policy: ServicePolicy,
}

impl ExternalService {
    pub fn new(library: Library, policy: ServicePolicy) -> Self {
        Self { library, policy }
    }

    pub fn policy(&self) -> ServicePolicy {
        self.policy
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    /// Compatibility entry point for local adapters and unit tests. Production
    /// adapters should use `dispatch_as` so the audit principal is explicit.
    pub fn dispatch(
        &mut self,
        request: &ExternalRequest,
        granted: &BTreeSet<ExternalScope>,
    ) -> ExternalResponse {
        self.dispatch_as(request, &ExternalGrant::mcp(granted.clone()))
    }

    pub fn dispatch_as(
        &mut self,
        request: &ExternalRequest,
        grant: &ExternalGrant,
    ) -> ExternalResponse {
        let started = Instant::now();
        let response = self.execute(request, &grant.scopes);
        let outcome = if response.ok {
            ExternalAuditOutcome::Allowed
        } else if matches!(
            response.error.as_ref().map(|error| error.code),
            Some(ExternalErrorCode::Unauthorized | ExternalErrorCode::Forbidden)
        ) {
            ExternalAuditOutcome::Denied
        } else {
            ExternalAuditOutcome::Failed
        };
        let audit = ExternalAuditDraft {
            request_id: request.request_id.clone(),
            transport: grant.transport.clone(),
            principal_id: grant.principal_id.clone(),
            method: request.command.method_name().into(),
            required_scope: request.command.required_scope(),
            granted_scopes: grant.scopes.iter().copied().collect(),
            outcome,
            error_code: response.error.as_ref().map(|error| error.code),
            subject_id: subject_id(&request.command),
            duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        };
        match self.library.record_external_audit(audit) {
            Ok(()) => response,
            Err(_) => failure(
                &request.request_id,
                ExternalErrorCode::Unavailable,
                "external audit is unavailable",
            ),
        }
    }

    /// Records a request rejected before dispatch. No token value or request
    /// body is accepted by this API.
    pub fn record_auth_failure(
        &self,
        request: &ExternalRequest,
        principal_id: &str,
        code: ExternalErrorCode,
    ) -> pkms_core::Result<()> {
        self.library.record_external_audit(ExternalAuditDraft {
            request_id: request.request_id.clone(),
            transport: "http".into(),
            principal_id: principal_id.into(),
            method: request.command.method_name().into(),
            required_scope: request.command.required_scope(),
            granted_scopes: Vec::new(),
            outcome: ExternalAuditOutcome::Denied,
            error_code: Some(code),
            subject_id: subject_id(&request.command),
            duration_ms: 0,
        })
    }

    fn execute(
        &mut self,
        request: &ExternalRequest,
        granted: &BTreeSet<ExternalScope>,
    ) -> ExternalResponse {
        if let Err(message) = request.validate() {
            return failure(
                &request.request_id,
                ExternalErrorCode::InvalidRequest,
                message,
            );
        }
        let required = request.command.required_scope();
        if !granted.contains(&required) {
            return ExternalResponse::failure(
                request.request_id.clone(),
                ExternalError {
                    code: ExternalErrorCode::Forbidden,
                    message: "the client does not have the required scope".into(),
                    required_scope: Some(required),
                    retryable: false,
                },
            );
        }
        match &request.command {
            ExternalCommand::Capabilities => {
                let mut capabilities = ExternalCapabilities::v1();
                capabilities.methods.retain(|method| match method.as_str() {
                    "propose_note" => self.policy.allow_proposals,
                    "audit_page" => self.policy.allow_audit,
                    _ => true,
                });
                success(&request.request_id, capabilities)
            }
            ExternalCommand::Search(params) => self.search(&request.request_id, params),
            ExternalCommand::ReadNote(params) => self.read_note(&request.request_id, params),
            ExternalCommand::ListNotes(params) => self.list_notes(&request.request_id, params),
            ExternalCommand::ProposeNote(params) => {
                if !self.policy.allow_proposals {
                    return failure(
                        &request.request_id,
                        ExternalErrorCode::Forbidden,
                        "proposal tools are disabled for this server",
                    );
                }
                self.propose_note(&request.request_id, params)
            }
            ExternalCommand::AuditPage(params) => {
                if !self.policy.allow_audit {
                    return failure(
                        &request.request_id,
                        ExternalErrorCode::Forbidden,
                        "audit tools are disabled for this server",
                    );
                }
                self.audit_page(&request.request_id, params)
            }
        }
    }

    fn search(&self, request_id: &str, params: &ExternalSearchParams) -> ExternalResponse {
        if params.mode != SearchMode::Lexical {
            return failure(
                request_id,
                ExternalErrorCode::Unavailable,
                "external search currently supports lexical mode only",
            );
        }
        match self.library.keyword_search(&params.clone().into()) {
            Ok(results) => success(request_id, results),
            Err(error) => core_failure(request_id, error),
        }
    }

    fn read_note(&self, request_id: &str, params: &ExternalReadNoteParams) -> ExternalResponse {
        let result = match &params.revision_id {
            Some(revision_id) => self.library.revision(&params.note_id, revision_id),
            None => self.library.note(&params.note_id),
        };
        match result {
            Ok(note) => success(request_id, note),
            Err(error) => core_failure(request_id, error),
        }
    }

    fn list_notes(&self, request_id: &str, params: &ExternalListNotesParams) -> ExternalResponse {
        match self.library.notes_page(NotesPageRequest {
            limit: params.limit,
            query: params.query.clone(),
            cursor: params.cursor.clone(),
            trashed: params.trashed,
            filter: None,
            sort: None,
        }) {
            Ok(page) => success(request_id, page),
            Err(error) => core_failure(request_id, error),
        }
    }

    fn propose_note(
        &mut self,
        request_id: &str,
        params: &ExternalProposeNoteParams,
    ) -> ExternalResponse {
        let proposal = CreateProposal {
            answer_id: None,
            chat_run_id: None,
            external_request_id: Some(request_id.to_owned()),
            note_id: params.note_id.clone(),
            base_revision_id: params.base_revision_id.clone(),
            title: params.title.clone(),
            document_schema: params.document_schema,
            document: params.document.clone(),
        };
        match self
            .library
            .create_proposal(&uuid::Uuid::new_v4().to_string(), proposal)
        {
            Ok(created) => success(request_id, created.proposal),
            Err(error) => core_failure(request_id, error),
        }
    }

    fn audit_page(&self, request_id: &str, params: &ExternalAuditPageParams) -> ExternalResponse {
        match self.library.external_audit_page(params.clone()) {
            Ok(page) => success(request_id, page),
            Err(error) => core_failure(request_id, error),
        }
    }
}

fn subject_id(command: &ExternalCommand) -> Option<String> {
    match command {
        ExternalCommand::ReadNote(params) => Some(params.note_id.clone()),
        ExternalCommand::ProposeNote(params) => params.note_id.clone(),
        ExternalCommand::Search(params) => params.scope.collection_id.clone(),
        ExternalCommand::ListNotes(_)
        | ExternalCommand::Capabilities
        | ExternalCommand::AuditPage(_) => None,
    }
}

fn success(request_id: &str, value: impl serde::Serialize) -> ExternalResponse {
    match serde_json::to_value(value) {
        Ok(data) => ExternalResponse::success(request_id.to_owned(), data),
        Err(_) => failure(
            request_id,
            ExternalErrorCode::Internal,
            "response encoding failed",
        ),
    }
}

fn failure(request_id: &str, code: ExternalErrorCode, message: &str) -> ExternalResponse {
    ExternalResponse::failure(
        request_id.to_owned(),
        ExternalError {
            code,
            message: message.into(),
            required_scope: None,
            retryable: false,
        },
    )
}

fn core_failure(request_id: &str, source: pkms_core::Error) -> ExternalResponse {
    let (code, message) = match source {
        pkms_core::Error::NotFound => (ExternalErrorCode::NotFound, "object not found"),
        pkms_core::Error::Conflict { .. }
        | pkms_core::Error::RevisionConflict { .. }
        | pkms_core::Error::MetadataConflict { .. }
        | pkms_core::Error::Trashed => {
            (ExternalErrorCode::Conflict, "revision or metadata conflict")
        }
        pkms_core::Error::InvalidInput(message) | pkms_core::Error::InvalidDocument(message) => {
            (ExternalErrorCode::InvalidRequest, message)
        }
        pkms_core::Error::UnsupportedDatabase
        | pkms_core::Error::WorkspaceBusy
        | pkms_core::Error::MigrationHistoryMismatch { .. } => {
            (ExternalErrorCode::Unavailable, "workspace is unavailable")
        }
        _ => (ExternalErrorCode::Internal, "internal failure"),
    };
    failure(request_id, code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pkms_core::{
        ExternalCommand, ExternalErrorCode, ExternalRequest, ExternalSearchParams, NoteDocument,
        SearchScope,
    };
    use std::collections::BTreeSet;

    fn scopes(values: &[ExternalScope]) -> BTreeSet<ExternalScope> {
        values.iter().copied().collect()
    }

    fn workspace() -> (tempfile::TempDir, ExternalService, String) {
        let directory = tempfile::tempdir().unwrap();
        let mut library = Library::open(directory.path()).unwrap();
        let note = library
            .create_note("外部接口测试", "知识管理与检索证据")
            .unwrap();
        (
            directory,
            ExternalService::new(library, ServicePolicy::default()),
            note.id,
        )
    }

    fn search_request() -> ExternalRequest {
        ExternalRequest {
            protocol: pkms_core::EXTERNAL_PROTOCOL.into(),
            request_id: "read-search-1".into(),
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
    fn read_only_search_and_read_do_not_change_the_note() {
        let (_directory, mut service, note_id) = workspace();
        let before = service.library().note(&note_id).unwrap();
        let search = service.dispatch(
            &search_request(),
            &scopes(&[ExternalScope::Read, ExternalScope::Search]),
        );
        assert!(search.ok, "{:?}", search.error);
        let read = ExternalRequest {
            protocol: pkms_core::EXTERNAL_PROTOCOL.into(),
            request_id: "read-1".into(),
            command: ExternalCommand::ReadNote(pkms_core::ExternalReadNoteParams {
                note_id: note_id.clone(),
                revision_id: None,
            }),
        };
        assert!(service.dispatch(&read, &scopes(&[ExternalScope::Read])).ok);
        let after = service.library().note(&note_id).unwrap();
        assert_eq!(before.revision_id, after.revision_id);
        assert_eq!(before.content_hash, after.content_hash);
    }

    #[test]
    fn default_server_rejects_proposal_scope_and_does_not_write() {
        let (_directory, mut service, note_id) = workspace();
        let before = service.library().note(&note_id).unwrap();
        let document = NoteDocument::from_text(uuid::Uuid::new_v4().to_string(), "提案正文");
        let request = ExternalRequest {
            protocol: pkms_core::EXTERNAL_PROTOCOL.into(),
            request_id: "proposal-1".into(),
            command: ExternalCommand::ProposeNote(pkms_core::ExternalProposeNoteParams {
                note_id: Some(note_id.clone()),
                base_revision_id: Some(before.revision_id.clone()),
                title: "外部提案".into(),
                document_schema: pkms_core::DOCUMENT_SCHEMA,
                document,
            }),
        };
        let response = service.dispatch(
            &request,
            &scopes(&[ExternalScope::Read, ExternalScope::Search]),
        );
        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ExternalErrorCode::Forbidden)
        );
        let after = service.library().note(&note_id).unwrap();
        assert_eq!(before.revision_id, after.revision_id);
    }

    #[test]
    fn denied_request_is_auditable_by_request_principal_and_subject() {
        let (_directory, mut service, note_id) = workspace();
        let note = service.library().note(&note_id).unwrap();
        let request = ExternalRequest {
            protocol: pkms_core::EXTERNAL_PROTOCOL.into(),
            request_id: "denied-audit-1".into(),
            command: ExternalCommand::ProposeNote(pkms_core::ExternalProposeNoteParams {
                note_id: Some(note_id.clone()),
                base_revision_id: Some(note.revision_id),
                title: "不允许的提案".into(),
                document_schema: pkms_core::DOCUMENT_SCHEMA,
                document: note.document,
            }),
        };
        let response = service.dispatch(
            &request,
            &scopes(&[ExternalScope::Read, ExternalScope::Search]),
        );
        assert_eq!(
            response.error.as_ref().map(|error| error.code),
            Some(ExternalErrorCode::Forbidden)
        );
        let page = service
            .library()
            .external_audit_page(ExternalAuditPageParams {
                limit: 10,
                before_seq: None,
                request_id: Some("denied-audit-1".into()),
                principal_id: Some("mcp-local".into()),
                subject_id: Some(note_id),
            })
            .unwrap();
        assert_eq!(page.items.len(), 1);
        let event = &page.items[0];
        assert_eq!(event.outcome, ExternalAuditOutcome::Denied);
        assert_eq!(event.method, "propose_note");
        assert_eq!(event.required_scope, ExternalScope::Propose);
        assert_eq!(event.error_code, Some(ExternalErrorCode::Forbidden));
    }

    #[test]
    fn authentication_failure_is_recorded_without_a_token_value() {
        let (_directory, service, _note_id) = workspace();
        service
            .record_auth_failure(
                &search_request(),
                "unauthenticated",
                ExternalErrorCode::Unauthorized,
            )
            .unwrap();
        let page = service
            .library()
            .external_audit_page(ExternalAuditPageParams {
                limit: 10,
                before_seq: None,
                request_id: Some("read-search-1".into()),
                principal_id: Some("unauthenticated".into()),
                subject_id: None,
            })
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].transport, "http");
        assert_eq!(page.items[0].outcome, ExternalAuditOutcome::Denied);
        assert_eq!(page.items[0].granted_scopes, Vec::<ExternalScope>::new());
    }
}

//! Transport DTOs and dispatch shared by the native adapter and contract tests.
use crate::asr::AsrJob;
use crate::*;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use pkms_core::{
    AssignTag, AttachmentImport, BlockHistoryPage, BlockHistoryRequest, BlockPage,
    BlocksPageRequest, CopyNote, CreateNote, MetadataCommit, Note, NoteCommit, NotePage,
    NotesPageRequest, ObjectMetadata, RemoveTag, RestoreNote, RevisionPage, SaveNote,
    SetInboxState, TagPage, TagsPageRequest, WorkspaceInfo,
};
use pkms_core::{
    CanvasCommit, CanvasPage, CanvasPageRequest, CanvasView, ChangeCollectionMembers,
    CollectionCommit, CollectionDetail, CollectionMembersPage, CollectionMembersRequest,
    CollectionPage, CollectionPageRequest, CreateCollection, RemoveCanvas, RemoveView, SaveCanvas,
    SaveView, SavedView, SetObjectFlag, UpdateCollection, ViewCommit, ViewPage, ViewPageRequest,
    ViewRun,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Transient IPC input, never part of a persisted command receipt or profile.
#[derive(Clone, Serialize, Deserialize, TS)]
#[serde(transparent)]
pub struct ApiCredential(String);
impl std::fmt::Debug for ApiCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiCredential([redacted])")
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct GenerationCredentialStatus {
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AsrSettingsSave {
    pub request: pkms_core::UpdateAsrSettings,
    #[serde(default)]
    pub credential: Option<ApiCredential>,
    #[serde(default)]
    pub clear_credential: bool,
}

fn legacy_generation_credential_binding(
    identity: &WorkspaceIdentity,
    profile: &pkms_core::GenerationProfile,
) -> Result<crate::inference_credentials::CredentialBinding> {
    // Channel credentials require a saved channel revision. The pre-channel
    // profile commands cannot bypass that check by carrying a channel ID.
    if profile.channel_id.is_some() {
        return Err(AppError::InvalidInput(
            "generation_channel_version_required",
        ));
    }
    crate::inference_remote::credential_binding(identity, profile)
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DesktopRequest {
    #[ts(type = "1")]
    pub api_version: u16,
    pub request_id: String,
    pub identity: Option<WorkspaceIdentity>,
    pub command: DesktopCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DesktopCommand {
    WorkspaceStatistics,
    DiagnosticsPreview {
        categories: Vec<pkms_core::DiagnosticCategory>,
    },
    DiagnosticsExport {
        command_id: String,
        token: String,
        categories: Vec<pkms_core::DiagnosticCategory>,
        fingerprint: String,
    },
    ReviewList(pkms_core::ReviewListRequest),
    ReviewCounts {
        collection_id: Option<String>,
    },
    ReviewGet {
        entry_id: String,
    },
    ReviewHistory(pkms_core::ReviewHistoryRequest),
    ReviewAdd {
        command_id: String,
        request: pkms_core::AddReview,
    },
    ReviewComplete {
        command_id: String,
        request: pkms_core::ReviewChange,
    },
    ReviewSnooze {
        command_id: String,
        request: pkms_core::SnoozeReview,
    },
    ReviewRemove {
        command_id: String,
        request: pkms_core::ReviewChange,
    },
    AutomationChangeSetsPage(pkms_core::ChangeSetPageRequest),
    AutomationChangeSetGet {
        change_id: String,
    },
    AutomationCreateChangeSet {
        command_id: String,
        request: pkms_core::CreateChangeSet,
    },
    AutomationApplyItem {
        command_id: String,
        request: pkms_core::ApplyChangeItem,
    },
    AutomationSkipItem {
        command_id: String,
        request: pkms_core::ApplyChangeItem,
    },
    AutomationRejectChangeSet {
        command_id: String,
        request: pkms_core::RejectChangeSet,
    },
    AutomationExpire,
    AutomationStaleReport {
        window_days: u32,
        limit: u32,
    },
    AuditPage(pkms_core::AuditPageRequest),
    AuditUndo {
        command_id: String,
        request: pkms_core::UndoAuditEvent,
    },
    ResearchCreate {
        command_id: String,
        request: pkms_core::CreateResearchTask,
    },
    ResearchGet {
        task_id: String,
    },
    ResearchPage(pkms_core::ResearchPageRequest),
    ResearchAdvance {
        command_id: String,
        request: pkms_core::AdvanceResearch,
    },
    ResearchCancel {
        command_id: String,
        request: pkms_core::CancelResearch,
    },
    ResearchRunExternal {
        command_id: String,
        task_id: String,
        base_row_version: u32,
    },
    ReviewInsightsGet {
        limit: u32,
    },
    ReviewInsightSettingsGet,
    ReviewInsightSettingsSet {
        command_id: String,
        request: pkms_core::SetReviewInsightSettings,
    },
    AppState,
    Capabilities,
    WorkspacesList,
    WorkspaceSelect {
        target: WorkspaceIdentity,
    },
    WorkspaceForget,
    WorkspaceClose,
    PreferencesGet,
    PreferencesUpdate {
        base_state_version: u32,
        preferences: AppPreferences,
    },
    WorkspaceMetadataGet,
    WorkspaceMetadataUpdate {
        request: pkms_core::UpdateWorkspaceMetadata,
    },
    VoiceCaptures {
        trashed: bool,
    },
    VoiceBrowse(pkms_core::VoiceBrowseRequest),
    VoiceCaptureGet {
        capture_id: String,
    },
    VoiceSyncTasks,
    VoiceCaptureCreate {
        command_id: String,
        request: pkms_core::CreateVoiceCapture,
        audio_base64: String,
    },
    VoiceCaptureUpdate {
        request: pkms_core::UpdateVoiceCapture,
    },
    VoiceCaptureTrash {
        request: pkms_core::VoiceCaptureTrashRequest,
    },
    VoiceCaptureSaveNote {
        command_id: String,
        request: pkms_core::SaveVoiceCaptureNote,
    },
    VoiceCaptureTranscribe {
        capture_id: String,
        base_row_version: Option<u32>,
    },
    VoiceCaptureSummarize {
        request: pkms_core::VoiceSummaryRequest,
    },
    AsrSettingsGet,
    AsrSettingsSave(AsrSettingsSave),
    AsrSettingsTest,
    SessionGet,
    SessionSave {
        base_state_version: u32,
        layout: SessionLayout,
    },
    RecentObjectsList,
    RecentObjectsTouch {
        object_id: String,
    },
    WorkspaceCreate {
        command_id: String,
        token: String,
    },
    WorkspaceOpen {
        command_id: String,
        token: String,
    },
    BackupInspect {
        token: String,
    },
    BackupVerify {
        command_id: String,
        token: String,
    },
    BackupRestore {
        command_id: String,
        token: String,
        output_token: String,
    },
    BackupCreate {
        command_id: String,
        token: String,
    },
    HandoffExport {
        token: String,
    },
    HandoffImport {
        token: String,
    },
    SyncStatus,
    VoiceSyncImport,
    SyncConfigure {
        endpoint: String,
        display_name: String,
    },
    SyncPair {
        invite_id: String,
        code: String,
    },
    SyncOwnerProof,
    SyncClaimDevice {
        device_id: String,
    },
    SyncCreateInvite {
        role: SyncMemberRole,
        member_name: String,
    },
    SyncCreateDeviceInvite {
        member_id: String,
        role: SyncMemberRole,
    },
    SyncSetDeviceRole {
        target_device_id: String,
        role: SyncMemberRole,
    },
    SyncMembers,
    SyncAudit {
        limit: u32,
    },
    SyncSetMemberRole {
        member_id: String,
        role: SyncMemberRole,
    },
    SyncRemoveMember {
        member_id: String,
        reason: String,
    },
    SyncDevices,
    SyncRevokeDevice {
        target_device_id: String,
        reason: String,
    },
    SyncAttachWorkspace {
        workspace_id: String,
    },
    SyncEnsureWorkspaceKey {
        workspace_id: String,
    },
    SyncGrantWorkspaceKey {
        workspace_id: String,
        target_device_id: String,
    },
    SyncPushNote {
        note_id: String,
    },
    SyncAutoPushNote {
        note_id: String,
    },
    SyncAutoPushWorkspaceMetadata,
    SyncAutoPushTags,
    SyncPushAll,
    SyncPull,
    SyncTagPage(pkms_core::TagSyncPageRequest),
    SyncTagRefresh {
        kind: pkms_core::TagSyncKind,
        entity_id: String,
    },
    SyncTagKeepLocal(SyncTagDecision),
    SyncTagKeepRemote(SyncTagDecision),
    SyncConfigConflicts,
    SyncWatch {
        timeout_ms: u32,
    },
    SyncResolveKeepLocal {
        conflict: SyncConflictTarget,
    },
    SyncResolveKeepRemote {
        conflict: SyncConflictTarget,
    },
    SyncResolveSaveAsNew {
        conflict: SyncConflictTarget,
    },
    SyncConfigKeepLocal {
        object_id: String,
        #[ts(type = "number")]
        remote_version: i64,
        local_state_token: String,
    },
    SyncConfigKeepRemote {
        object_id: String,
        #[ts(type = "number")]
        remote_version: i64,
        local_state_token: String,
    },
    SyncDeleteNote {
        note_id: String,
    },
    SyncRestoreNote {
        note_id: String,
    },
    SyncTrash,
    NoteExport {
        command_id: String,
        token: String,
        note_id: String,
        revision_id: String,
        format: pkms_core::ExportFormat,
        #[serde(default)]
        citation_style_id: Option<String>,
    },
    /// Exports one immutable document original to a user-chosen destination.
    SourceExport {
        command_id: String,
        token: String,
        source_id: String,
        version_id: String,
    },
    JobsList {
        limit: u32,
    },
    JobsPage {
        limit: u32,
        cursor: Option<String>,
    },
    JobGet {
        job_id: String,
    },
    JobCancel {
        job_id: String,
    },
    OperationsList {
        limit: u32,
    },
    OperationsPage {
        limit: u32,
        cursor: Option<String>,
    },
    OperationGet {
        operation_id: String,
    },
    OperationCancel {
        operation_id: String,
    },
    WorkspaceInfo,
    ObjectsMetadataGet {
        object_id: String,
    },
    TrashList(pkms_core::TrashPageRequest),
    TrashPreview(pkms_core::TrashPreviewRequest),
    TrashDelete {
        command_id: String,
        request: pkms_core::DeleteTrash,
    },
    TagsList(TagsPageRequest),
    TagsBrowse(pkms_core::TagsBrowseRequest),
    TaggedObjects(pkms_core::TaggedObjectsRequest),
    TagsGet {
        tag_id: String,
    },
    TagsCreate {
        command_id: String,
        request: pkms_core::CreateTag,
    },
    TagsUpdate {
        command_id: String,
        request: pkms_core::UpdateTag,
    },
    TagsDelete {
        command_id: String,
        request: pkms_core::DeleteTag,
    },
    TagsAssign {
        command_id: String,
        request: AssignTag,
    },
    TagsRemove {
        command_id: String,
        request: RemoveTag,
    },
    ObjectsSetInboxState {
        command_id: String,
        request: SetInboxState,
    },
    ObjectsSetTrashed {
        command_id: String,
        request: SetObjectFlag,
    },
    ObjectsSetArchived {
        command_id: String,
        request: SetObjectFlag,
    },
    ObjectsSetSearchEnabled {
        command_id: String,
        request: SetObjectFlag,
    },
    CollectionsGet {
        collection_id: String,
    },
    CollectionsList(CollectionPageRequest),
    CollectionsMembers(CollectionMembersRequest),
    CollectionsCreate {
        command_id: String,
        request: CreateCollection,
    },
    CollectionsUpdate {
        command_id: String,
        request: UpdateCollection,
    },
    CollectionsAddMembers {
        command_id: String,
        request: ChangeCollectionMembers,
    },
    CollectionsRemoveMembers {
        command_id: String,
        request: ChangeCollectionMembers,
    },
    CollectionsReorder {
        command_id: String,
        request: ChangeCollectionMembers,
    },
    ViewsGet {
        view_id: String,
    },
    ViewsList(ViewPageRequest),
    ViewsRun {
        view_id: String,
        limit: u32,
        cursor: Option<String>,
    },
    ViewsSave {
        command_id: String,
        request: SaveView,
    },
    ViewsRemove {
        command_id: String,
        request: RemoveView,
    },
    CanvasesGet {
        canvas_id: String,
    },
    CanvasesList(CanvasPageRequest),
    CanvasesSave {
        command_id: String,
        request: SaveCanvas,
    },
    CanvasesRemove {
        command_id: String,
        request: RemoveCanvas,
    },
    KnowledgeObjectsList(pkms_core::KnowledgeObjectPageRequest),
    KnowledgeBrowse(pkms_core::KnowledgeBrowseRequest),
    NotesList(NotesPageRequest),
    SearchQuery(pkms_core::SearchQuery),
    EmbeddingProfileGet,
    EmbeddingProfileRegister {
        command_id: String,
    },
    EmbeddingRemoteConfigure {
        command_id: String,
        provider: pkms_core::EmbeddingProvider,
        endpoint: String,
        model: String,
        output_dimensions: Option<u32>,
    },
    EmbeddingCredentialSet {
        provider: pkms_core::EmbeddingProvider,
        endpoint: String,
        model: String,
        #[ts(type = "string")]
        api_key: ApiCredential,
    },
    EmbeddingCredentialStatus {
        provider: pkms_core::EmbeddingProvider,
        endpoint: String,
        model: String,
    },
    EmbeddingCredentialDelete {
        provider: pkms_core::EmbeddingProvider,
        endpoint: String,
        model: String,
    },
    IndexStatus,
    IndexRebuildStart {
        command_id: String,
        request: pkms_core::RebuildRequest,
    },
    IndexRebuildGet {
        job_id: String,
    },
    IndexRebuildList {
        after_id: Option<String>,
        limit: u32,
    },
    IndexRebuildCancel {
        command_id: String,
        job_id: String,
    },
    GenerationSettingsGet,
    RetrievalSettingsGet,
    CleaningSettingsGet,
    ChunkSettingsGet,
    EmbeddingProfilePrevious,
    GenerationChannels,
    GenerationChannelSave {
        command_id: String,
        request: pkms_core::SaveGenerationChannel,
    },
    GenerationChannelDelete {
        command_id: String,
        request: pkms_core::DeleteGenerationChannel,
    },
    GenerationChannelSelect {
        command_id: String,
        request: pkms_core::SelectGenerationChannelModel,
    },
    GenerationChannelCredentialStatus {
        channel_id: String,
        row_version: u32,
    },
    GenerationChannelCredentialSet {
        channel_id: String,
        row_version: u32,
        #[ts(type = "string")]
        api_key: ApiCredential,
    },
    GenerationChannelCredentialDelete {
        channel_id: String,
        row_version: u32,
    },
    GenerationChannelModelsList {
        channel_id: String,
        row_version: u32,
    },
    GenerationCredentialStatus {
        profile: pkms_core::GenerationProfile,
    },
    GenerationCredentialSet {
        profile: pkms_core::GenerationProfile,
        #[ts(type = "string")]
        api_key: ApiCredential,
    },
    GenerationCredentialDelete {
        profile: pkms_core::GenerationProfile,
    },
    GenerationModelsList {
        endpoint: String,
    },
    GenerationModelVerify {
        profile: pkms_core::GenerationProfile,
    },
    GenerationSettingsSet {
        command_id: String,
        request: pkms_core::SetGenerationProfile,
    },
    RetrievalSettingsSet {
        command_id: String,
        request: pkms_core::SetRetrievalParams,
    },
    CleaningSettingsSet {
        command_id: String,
        request: pkms_core::SetCleaningParams,
    },
    ChunkSettingsSet {
        command_id: String,
        request: pkms_core::SetChunkParams,
    },
    EmbeddingProfileRevert {
        command_id: String,
    },
    ChatConversationsCreate {
        command_id: String,
        request: pkms_core::CreateConversation,
    },
    ChatConversationsList {
        after_id: Option<String>,
        limit: u32,
    },
    ChatConversationsPage {
        before: Option<pkms_core::ChatConversationCursor>,
        limit: u32,
    },
    ChatMessagesPage {
        conversation_id: String,
        before_ordinal: Option<u32>,
        limit: u32,
    },
    ChatConversationsGet {
        conversation_id: String,
    },
    ChatMessagesList {
        conversation_id: String,
        after_ordinal: u32,
        limit: u32,
    },
    ChatRunsGet {
        run_id: String,
    },
    ChatRunsEvents {
        run_id: String,
        after_seq: u32,
        limit: u32,
    },
    ChatRunsStart {
        command_id: String,
        request: pkms_core::StartChatRun,
    },
    ChatRunsCancel {
        command_id: String,
        run_id: String,
    },
    KnowledgeTasksStart {
        command_id: String,
        request: pkms_core::StartKnowledgeTask,
    },
    KnowledgeTasksGet {
        task_id: String,
    },
    KnowledgeTasksPage(pkms_core::KnowledgeTasksRequest),
    KnowledgeTaskMaterials(pkms_core::KnowledgeTaskMaterialsRequest),
    KnowledgeTaskOrigins(pkms_core::KnowledgeTaskOriginsRequest),
    KnowledgeTaskOutputsPage(pkms_core::KnowledgeTaskOutputsRequest),
    KnowledgeTaskTemplatesGet {
        template_id: String,
    },
    KnowledgeTaskTemplatesPage(pkms_core::KnowledgeTaskTemplatesRequest),
    KnowledgeTaskTemplatesSave {
        command_id: String,
        request: pkms_core::SaveKnowledgeTaskTemplate,
    },
    KnowledgeTaskTemplatesDelete {
        command_id: String,
        request: pkms_core::DeleteKnowledgeTaskTemplate,
    },
    AnswersStart {
        command_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional = nullable)]
        max_output_tokens: Option<u32>,
        search: pkms_core::SearchQuery,
    },
    AnswersAgentStart {
        command_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional = nullable)]
        max_output_tokens: Option<u32>,
        search: pkms_core::SearchQuery,
        agent: pkms_core::AgentOptions,
    },
    AnswersGet {
        answer_id: String,
    },
    AnswersList {
        after_id: Option<String>,
        limit: u32,
    },
    AnswersEvents {
        answer_id: String,
        after_seq: u32,
        limit: u32,
    },
    AnswersCancel {
        command_id: String,
        answer_id: String,
    },
    AnswersFeedback {
        command_id: String,
        request: pkms_core::AnswerFeedback,
    },
    /// The collected answer ratings, for the feedback report.
    FeedbackList {
        limit: u32,
        cursor: Option<String>,
    },
    FeedbackSummary,
    ProposalsCreate {
        command_id: String,
        request: pkms_core::CreateProposal,
    },
    ProposalsGet {
        proposal_id: String,
    },
    ProposalsApply {
        command_id: String,
        proposal_id: String,
    },
    ProposalsReject {
        command_id: String,
        proposal_id: String,
    },
    SourcesList(pkms_core::SourcesPageRequest),
    SourcesBrowse(pkms_core::SourcesBrowseRequest),
    SourcesGet {
        source_id: String,
    },
    SourcesVersion {
        source_version_id: String,
    },
    SourcesNoteInput {
        source_version_id: String,
    },
    SourcesCreateNote {
        command_id: String,
        request: pkms_core::CreateSourceNote,
    },
    SourcesVersions {
        source_id: String,
        after_id: Option<String>,
        limit: u32,
    },
    SourcesText {
        source_version_id: String,
        parse_id: Option<String>,
        offset: u32,
        limit: u32,
    },
    SourcesCover {
        source_version_id: String,
        parse_id: Option<String>,
    },
    SourcesImage {
        source_version_id: String,
        parse_id: String,
        blob_hash: String,
    },
    SourcesSearch {
        query: String,
        limit: u32,
    },
    SourcesImport {
        command_id: String,
        request: pkms_core::ImportSource,
        #[serde(default)]
        tag_names: Vec<String>,
    },
    SourcesImportWebpage {
        command_id: String,
        request: pkms_core::ImportWebpage,
        #[serde(default)]
        tag_names: Vec<String>,
    },
    /// Reads a user-granted directory and returns one grant per importable file.
    /// Read-only for the library; the granted tokens are the only new state.
    SourceDirectoryScan {
        token: String,
    },
    SourcesUpdate {
        command_id: String,
        request: pkms_core::UpdateSourceMetadata,
    },
    SourcesSetTrashed {
        command_id: String,
        request: SetObjectFlag,
    },
    AnnotationsGet {
        id: String,
    },
    AnnotationsQuery(pkms_core::AnnotationsQueryRequest),
    AnnotationsList {
        source_version_id: String,
        after_id: Option<String>,
        limit: u32,
        trashed: bool,
    },
    AnnotationsResolve {
        id: String,
    },
    AnnotationsCreate {
        command_id: String,
        request: pkms_core::CreateAnnotation,
    },
    AnnotationsUpdate {
        command_id: String,
        request: pkms_core::UpdateAnnotation,
    },
    AnnotationsSetTrashed {
        command_id: String,
        request: SetObjectFlag,
    },
    ReadingGet {
        source_id: String,
    },
    ReadingSet {
        command_id: String,
        request: pkms_core::SetReadingState,
    },
    SourceJobsList {
        limit: u32,
    },
    SourceJobsPage {
        limit: u32,
        cursor: Option<String>,
    },
    SourceJobGet {
        job_id: String,
    },
    SourceJobCancel {
        job_id: String,
    },
    SourceJobRetry {
        command_id: String,
        job_id: String,
    },
    SourceJobEvents {
        job_id: String,
        after_seq: u32,
        limit: u32,
    },
    NotesGet {
        note_id: String,
    },
    BlocksSearch(BlocksPageRequest),
    BlockHistory(BlockHistoryRequest),
    LinksResolve {
        input: pkms_core::ResolveLink,
    },
    KnowledgeGraph,
    NotesBacklinks {
        id: String,
        limit: u32,
        cursor: Option<String>,
    },
    NotesReferences {
        id: String,
        revision_id: String,
        limit: u32,
        cursor: Option<String>,
    },
    NotesRelationBacklinks {
        id: String,
        limit: u32,
        cursor: Option<String>,
    },
    NotesOutgoingRelations {
        id: String,
    },
    NotesCreate {
        command_id: String,
        note: CreateNote,
    },
    BibliographyCatalog,
    Bibliography {
        note_id: String,
        #[serde(default)]
        style: Option<pkms_core::CitationStyle>,
    },
    BibliographyImport {
        token: String,
    },
    CitationStyles,
    CitationStyleImport {
        token: String,
    },
    CitationStyleDelete {
        style_id: String,
    },
    BibliographyWithStyle {
        note_id: String,
        style_id: String,
    },
    NotesSave {
        command_id: String,
        note: SaveNote,
    },
    NotesCopy {
        command_id: String,
        note: CopyNote,
    },
    NotesRestore {
        command_id: String,
        note: RestoreNote,
    },
    RevisionsList {
        note_id: String,
        before: Option<u32>,
        limit: u32,
    },
    RevisionGet {
        note_id: String,
        revision_id: String,
    },
    AttachmentImport {
        command_id: String,
        token: String,
    },
    AttachmentResume {
        command_id: String,
    },
}

impl DesktopCommand {
    pub fn is_operation_control(&self) -> bool {
        matches!(
            self,
            Self::OperationsList { .. }
                | Self::OperationsPage { .. }
                | Self::OperationGet { .. }
                | Self::OperationCancel { .. }
                | Self::JobsList { .. }
                | Self::JobsPage { .. }
                | Self::JobGet { .. }
                | Self::JobCancel { .. }
                | Self::SourceJobsList { .. }
                | Self::SourceJobsPage { .. }
                | Self::SourceJobGet { .. }
                | Self::SourceJobCancel { .. }
                | Self::SourceJobEvents { .. }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopReply {
    pub request_id: String,
    pub data: DesktopData,
}

/// Events invalidate durable state; AgentProgress and ChatProgress are ephemeral runtime projections.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum DesktopEvent {
    AgentProgress {
        identity: WorkspaceIdentity,
        progress: crate::AgentProgress,
    },
    ChatProgress {
        identity: WorkspaceIdentity,
        progress: crate::ChatProgress,
    },
    ChatChanged {
        identity: WorkspaceIdentity,
        conversation_id: String,
        run_id: String,
        last_event_seq: u32,
    },
    InferenceChanged {
        identity: WorkspaceIdentity,
        answer_id: Option<String>,
        rebuild_id: Option<String>,
    },
    WorkspaceChanged {
        active: Option<WorkspaceIdentity>,
    },
    OperationChanged {
        operation_id: String,
    },
    OperationFailed {
        operation_id: String,
        error: ApiError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppState {
    pub active: Option<WorkspaceEntry>,
    pub preferences: PreferencesState,
    pub workspace_metadata: Option<pkms_core::WorkspaceMetadata>,
    pub workspaces: Vec<WorkspaceEntry>,
    /// Parse components this installation cannot use, with the exact path the
    /// application looked for (`worker_unavailable`, `pdfium_unavailable`). The
    /// interface shows them because retrying an import cannot repair an
    /// installation; the exported diagnostics bundle stays path-free.
    pub parse_components: Vec<crate::ParseComponentIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AppCapabilities {
    pub platform: String,
    pub document_schemas: Vec<u32>,
    pub image_formats: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct HandoffExportSummary {
    pub workspace_id: String,
    pub objects: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct HandoffImportSummary {
    pub imported: Vec<String>,
    pub skipped_existing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum DesktopData {
    WorkspaceStatistics(pkms_core::WorkspaceStatistics),
    DiagnosticsPreview(crate::DiagnosticsPreview),
    ReviewPage(pkms_core::ReviewPage),
    ReviewCounts(pkms_core::ReviewCounts),
    ReviewItem(pkms_core::ReviewItem),
    ReviewEventPage(pkms_core::ReviewEventPage),
    ReviewMutation(pkms_core::ReviewMutation),
    ChangeSetPage(pkms_core::ChangeSetPage),
    ChangeSet(pkms_core::ChangeSet),
    ChangeSetCommit(pkms_core::ChangeSetCommit),
    ChangeItemCommit(pkms_core::ChangeItemCommit),
    StaleSourceReport(pkms_core::StaleSourceReport),
    AuditPage(pkms_core::AuditPage),
    AuditUndoCommit(pkms_core::AuditUndoCommit),
    ResearchPage(pkms_core::ResearchPage),
    ResearchTask(pkms_core::ResearchTask),
    ResearchCommit(pkms_core::ResearchCommit),
    ReviewInsights(pkms_core::ReviewInsights),
    ReviewInsightSettings(pkms_core::ReviewInsightSettings),
    ReviewSettingsCommit(pkms_core::ReviewSettingsCommit),
    AppState(DesktopAppState),
    Capabilities(AppCapabilities),
    Workspaces(Vec<WorkspaceEntry>),
    BackupInspection(pkms_core::BackupInspection),
    HandoffExport(HandoffExportSummary),
    HandoffImport(HandoffImportSummary),
    SyncStatus(SyncStatus),
    VoiceSyncImport(VoiceSyncImport),
    SyncProfile(SyncProfile),
    SyncOwnerProof(SyncOwnerProof),
    SyncInvite(SyncInviteView),
    SyncPush(SyncPushSummary),
    SyncAutoPush(SyncAutoPushResult),
    SyncPushAll(SyncPushBatch),
    SyncTagPush(SyncTagPushBatch),
    SyncTagPage(pkms_core::TagSyncPage),
    SyncTagSnapshot(pkms_core::TagSyncSnapshot),
    SyncPull(SyncPullView),
    SyncConfigConflicts(Vec<SyncConfigConflictView>),
    SyncConfigResolution(SyncConfigResolution),
    SyncWatch(SyncWatchResult),
    SyncDevices(Vec<SyncDevice>),
    SyncDevice(SyncDevice),
    SyncMembers(Vec<SyncMember>),
    SyncMember(SyncMember),
    SyncAudit(Vec<SyncAuditEvent>),
    SyncMemberRemoval(SyncMemberRemoval),
    SyncObjectId(String),
    SyncTrash(Vec<SyncTrashItem>),
    JobAccepted(pkms_core::JobAccepted),
    Job(pkms_core::PortableJob),
    Jobs(Vec<pkms_core::PortableJob>),
    JobsPage(pkms_core::PortableJobPage),
    ResourceGrant(ResourceGrant),
    Preferences(PreferencesState),
    WorkspaceMetadata(pkms_core::WorkspaceMetadata),
    VoiceCapture(pkms_core::VoiceCapture),
    VoiceCaptures(pkms_core::VoiceCapturePage),
    VoiceBrowse(pkms_core::VoiceBrowsePage),
    VoiceSyncTasks(Vec<pkms_core::VoiceSyncTask>),
    VoiceSummary(pkms_core::VoiceSummary),
    AsrSettings(pkms_core::AsrSettingsState),
    AsrTestResult(pkms_core::AsrTestResult),
    Session(SessionState),
    RecentObjects(Vec<RecentObject>),
    Bootstrap(BootstrapAccepted),
    Operations(Vec<AppOperation>),
    OperationsPage(AppOperationPage),
    Operation(AppOperation),
    WorkspaceInfo(WorkspaceInfo),
    ObjectMetadata(ObjectMetadata),
    Tags(TagPage),
    TagsBrowsePage(pkms_core::TagsBrowsePage),
    TaggedObjectsPage(pkms_core::TaggedObjectsPage),
    TagDetails(pkms_core::TagDetails),
    TagManagementCommit(pkms_core::TagManagementCommit),
    MetadataCommit(MetadataCommit),
    Trash(pkms_core::TrashPage),
    TrashPreview(pkms_core::TrashPreview),
    TrashCommit(pkms_core::TrashCommit),
    Collection(CollectionDetail),
    Collections(CollectionPage),
    CollectionMembers(CollectionMembersPage),
    CollectionCommit(CollectionCommit),
    View(SavedView),
    Views(ViewPage),
    ViewRun(ViewRun),
    ViewCommit(ViewCommit),
    Canvas(CanvasView),
    Canvases(CanvasPage),
    CanvasCommit(CanvasCommit),
    KnowledgeObjects(pkms_core::KnowledgeObjectPage),
    KnowledgeBrowse(pkms_core::KnowledgeBrowsePage),
    Notes(NotePage),
    Blocks(BlockPage),
    BlockHistory(BlockHistoryPage),
    BibliographyCatalog(Vec<pkms_core::BibliographyEntry>),
    Bibliography(pkms_core::BibliographyReport),
    BibImport(pkms_core::BibImportReport),
    CitationStyles(Vec<pkms_core::CitationStyleSummary>),
    CitationStyleImport(pkms_core::CitationStyleImportReport),
    SearchResults(pkms_core::SearchResults),
    EmbeddingProfile(Option<pkms_core::EmbeddingProfile>),
    EmbeddingProfileMutation(pkms_core::EmbeddingProfileMutation),
    IndexStatus(pkms_core::IndexStatus),
    IndexRebuild(pkms_core::RebuildJob),
    IndexRebuilds(Vec<pkms_core::RebuildJob>),
    IndexRebuildMutation(pkms_core::RebuildMutation),
    GenerationSettings(pkms_core::GenerationSettings),
    RetrievalSettings(pkms_core::RetrievalSettings),
    CleaningSettings(pkms_core::CleaningSettings),
    ChunkSettings(pkms_core::ChunkSettings),
    GenerationChannels(pkms_core::GenerationChannelsState),
    GenerationChannelMutation(pkms_core::GenerationChannelMutation),
    RemoteGenerationModels(pkms_core::RemoteGenerationModels),
    GenerationCredentialStatus(GenerationCredentialStatus),
    GenerationModels(GenerationModels),
    AnnotationPage(pkms_core::AnnotationPage),
    GenerationSettingsMutation(pkms_core::GenerationSettingsMutation),
    RetrievalSettingsMutation(pkms_core::RetrievalSettingsMutation),
    CleaningSettingsMutation(pkms_core::CleaningSettingsMutation),
    ChunkSettingsMutation(pkms_core::ChunkSettingsMutation),
    ChatConversation(pkms_core::ChatConversation),
    ChatConversations(Vec<pkms_core::ChatConversation>),
    ChatConversationMutation(pkms_core::ChatConversationMutation),
    ChatMessages(Vec<pkms_core::ChatMessage>),
    ChatMessagePage(pkms_core::ChatMessagePage),
    ChatConversationPage(pkms_core::ChatConversationPage),
    ChatRun(pkms_core::ChatRun),
    ChatRunMutation(pkms_core::ChatRunMutation),
    KnowledgeTask(pkms_core::KnowledgeTask),
    KnowledgeTasksPage(pkms_core::KnowledgeTasksPage),
    KnowledgeTaskMaterials(pkms_core::KnowledgeTaskMaterials),
    KnowledgeTaskOrigins(pkms_core::KnowledgeTaskOriginsPage),
    KnowledgeTaskOutputsPage(pkms_core::KnowledgeTaskOutputsPage),
    KnowledgeTaskMutation(pkms_core::KnowledgeTaskMutation),
    KnowledgeTaskTemplate(pkms_core::KnowledgeTaskTemplate),
    KnowledgeTaskTemplatesPage(pkms_core::KnowledgeTaskTemplatesPage),
    KnowledgeTaskTemplateMutation(pkms_core::KnowledgeTaskTemplateMutation),
    ChatRunEvents(Vec<pkms_core::ChatRunEvent>),
    Answer(pkms_core::AnswerRecord),
    Answers(Vec<pkms_core::AnswerRecord>),
    AnswerEvents(Vec<pkms_core::AnswerEvent>),
    AnswerMutation(pkms_core::AnswerMutation),
    Feedback(pkms_core::FeedbackPage),
    FeedbackSummary(pkms_core::FeedbackSummary),
    Proposal(pkms_core::ProposalRecord),
    ProposalMutation(pkms_core::ProposalMutation),
    Sources(pkms_core::SourcePage),
    SourcesBrowsePage(pkms_core::SourcesBrowsePage),
    Source(pkms_core::SourceSummary),
    SourceVersion(pkms_core::SourceVersion),
    SourceNoteInput(pkms_core::SourceNoteInput),
    SourceVersions(Vec<pkms_core::SourceVersion>),
    SourceText(pkms_core::SourceTextPage),
    SourceCover(pkms_core::SourceCoverData),
    SourceImage(pkms_core::SourceImageData),
    SourceSearch(Vec<pkms_core::SourceSearchHit>),
    SourceMutation(Box<pkms_core::SourceMutation>),
    Annotation(pkms_core::Annotation),
    Annotations(Vec<pkms_core::Annotation>),
    ResolvedAnnotation(pkms_core::ResolvedAnnotation),
    Reading(pkms_core::ReadingState),
    SourceJobs(Vec<pkms_core::SourceJob>),
    SourceJobsPage(pkms_core::SourceJobPage),
    SourceJob(pkms_core::SourceJob),
    SourceJobEvents(Vec<pkms_core::SourceJobEvent>),
    SourceDirectoryScan(Box<crate::DirectoryScan>),
    Note(Note),
    ResolvedLink(pkms_core::ResolvedLink),
    KnowledgeGraph(pkms_core::KnowledgeGraph),
    BodyReferencePage(pkms_core::BodyReferencePage),
    RelationBacklinkPage(pkms_core::RelationBacklinkPage),
    OutgoingRelations(Vec<pkms_core::OutgoingRelation>),
    NoteCommit(NoteCommit),
    Revisions(RevisionPage),
    Attachment(AttachmentImport),
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    BackendStarting,
    QueryExpired,
    InvalidInput,
    InvalidDocument,
    UnsupportedApi,
    InstanceMismatch,
    AppStateConflict,
    RevisionConflict,
    MetadataConflict,
    IdempotencyMismatch,
    NotFound,
    ObjectTrashed,
    StorageBusy,
    StorageFull,
    IoFailed,
    IntegrityFailed,
    UnsupportedDatabase,
    MigrationFailed,
    InputUnavailable,
    OutputExists,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ApiErrorDetails {
    InferenceFailure {
        reason: String,
    },
    RevisionConflict {
        expected: String,
        actual: String,
    },
    MetadataConflict {
        object_id: String,
        expected: u32,
        actual: u32,
    },
    CitationFailure {
        reason: String,
    },
    SyncFailure {
        reason: String,
    },
    VoiceFailure {
        reason: String,
    },
    TagFailure {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
    pub request_id: String,
    pub retryable: bool,
    pub details: Option<ApiErrorDetails>,
}

impl ApiError {
    pub fn new(code: ApiErrorCode, request_id: &str) -> Self {
        let (message, retryable) = match code {
            ApiErrorCode::BackendStarting => ("正在准备本地存储，请稍候。", true),
            ApiErrorCode::QueryExpired => ("列表已更新，请刷新后继续浏览。", true),
            ApiErrorCode::InvalidInput => ("输入内容不符合要求，请检查后重试。", false),
            ApiErrorCode::InvalidDocument => (
                "正文包含不支持的结构或格式。草稿仍需保留，请检查内容。",
                false,
            ),
            ApiErrorCode::UnsupportedApi => ("应用界面与服务版本不兼容，请重新启动应用。", false),
            ApiErrorCode::InstanceMismatch => ("当前工作区已改变，请回到对应工作区后重试。", false),
            ApiErrorCode::AppStateConflict => ("界面设置已更新，请重新读取后再修改。", false),
            ApiErrorCode::RevisionConflict => {
                ("这篇笔记已有更新。请比较版本后决定如何保存。", false)
            }
            ApiErrorCode::MetadataConflict => {
                ("对象信息已有更新。请刷新后确认本次整理操作。", false)
            }
            ApiErrorCode::IdempotencyMismatch => ("本次操作的重试内容不一致，已停止提交。", false),
            ApiErrorCode::NotFound => ("未找到所需内容或授权已失效。", false),
            ApiErrorCode::ObjectTrashed => ("对象已在回收站，请先恢复再修改。", false),
            ApiErrorCode::StorageBusy => {
                ("工作区正被占用，请关闭其他使用此工作区的程序后重试。", true)
            }
            ApiErrorCode::StorageFull => ("磁盘空间不足。请保留草稿，释放空间后重试。", true),
            ApiErrorCode::IoFailed => ("无法读取或写入文件，请检查文件位置和访问权限。", true),
            ApiErrorCode::IntegrityFailed => {
                ("内容完整性检查未通过，请保留原文件并检查备份。", false)
            }
            ApiErrorCode::UnsupportedDatabase => {
                ("工作区格式不受当前版本支持，原数据未被替换。", false)
            }
            ApiErrorCode::MigrationFailed => {
                ("工作区升级检查未通过，请保留原库及升级前快照。", false)
            }
            ApiErrorCode::InputUnavailable => {
                ("所选文件或位置已改变，或授权已过期，请重新选择。", false)
            }
            ApiErrorCode::OutputExists => ("目标文件已存在。请换一个名称或保存位置。", false),
            ApiErrorCode::Internal => ("操作未完成，请保留当前内容并查看诊断信息。", false),
        };
        Self {
            code,
            message: message.into(),
            request_id: request_id.chars().take(64).collect(),
            retryable,
            details: None,
        }
    }

    pub fn from_app(error: AppError, request_id: &str) -> Self {
        if let AppError::InvalidInput(
            reason @ ("csl_required"
            | "csl_unreadable"
            | "csl_too_large"
            | "csl_not_utf8"
            | "csl_invalid_style"
            | "csl_invalid_identity"
            | "csl_invalid_parent"
            | "csl_builtin_id_reserved"
            | "csl_parent_missing"
            | "csl_parent_cycle"
            | "csl_parent_limit"
            | "csl_builtin_readonly"
            | "csl_style_in_use"),
        ) = &error
        {
            let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
            result.details = Some(ApiErrorDetails::CitationFailure {
                reason: (*reason).into(),
            });
            return result;
        }
        if let AppError::InvalidInput(
            reason @ ("answer_output_budget_invalid"
            | "embedding_remote_required"
            | "embedding_invalid_endpoint"
            | "embedding_dimension_mismatch"
            | "embedding_unavailable"
            | "embedding_api_failed"
            | "embedding_invalid_response"
            | "remote_authentication_failed"
            | "remote_rate_limited"
            | "model_unavailable"
            | "model_profile_unsupported"
            | "model_discovery_unsupported"
            | "model_template_unsupported"
            | "model_context_limit"
            | "model_response_limit"
            | "model_response_invalid"
            | "invalid_model_endpoint"
            | "model_invalid_endpoint"
            | "model_key_missing"
            | "model_invalid_key"
            | "model_credential_unavailable"
            | "model_credential_unsupported"
            | "model_authentication_failed"
            | "model_rate_limited"
            | "model_api_error"
            | "model_invalid_response"
            | "model_response_too_large"
            | "model_incomplete"
            | "model_unsupported_output"
            | "model_invalid_request"
            | "model_total_timeout"
            | "model_first_token_timeout"
            | "model_idle_timeout"
            | "remote_consent_required"
            | "semantic_disabled"),
        ) = &error
        {
            let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
            result.details = Some(ApiErrorDetails::InferenceFailure {
                reason: (*reason).into(),
            });
            return result;
        }
        if let AppError::InvalidInput(
            reason @ ("asr_not_configured"
            | "asr_credential_missing"
            | "asr_auth_failed"
            | "asr_rate_limited"
            | "asr_audio_too_large"
            | "asr_service_unavailable"
            | "asr_timeout"
            | "asr_network_error"
            | "asr_invalid_response"
            | "asr_request_rejected"
            | "asr_endpoint_invalid"
            | "asr_settings_incomplete"
            | "asr_settings_invalid"
            | "asr_already_running"
            | "asr_empty_transcript"
            | "asr_interrupted"
            | "asr_failed"
            | "asr_audio_invalid"
            | "asr_client_unavailable"
            | "model_profile_missing"
            | "model_key_missing"
            | "model_incomplete"
            | "voice_audio_invalid"
            | "voice_audio_too_large"
            | "voice_duration_too_long"
            | "voice_title_invalid"
            | "voice_language_invalid"
            | "voice_device_label_invalid"
            | "voice_transcript_empty"
            | "voice_transcript_too_long"
            | "sync_not_configured"
            | "sync_workspace_missing"
            | "sync_device_seed_missing"
            | "voice_sync_blob_empty"
            | "voice_sync_blob_invalid"
            | "voice_sync_hash_invalid"
            | "voice_sync_hash_mismatch"),
        ) = &error
        {
            let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
            result.details = Some(ApiErrorDetails::VoiceFailure {
                reason: (*reason).into(),
            });
            return result;
        }
        let code = match error {
            AppError::Core(error) => return Self::from_core(error, request_id),
            AppError::AppBusy => ApiErrorCode::StorageBusy,
            AppError::UnsupportedDatabase => ApiErrorCode::UnsupportedDatabase,
            AppError::StateConflict => ApiErrorCode::AppStateConflict,
            AppError::InstanceMismatch => ApiErrorCode::InstanceMismatch,
            AppError::NotFound => ApiErrorCode::NotFound,
            AppError::IdempotencyMismatch => ApiErrorCode::IdempotencyMismatch,
            AppError::InputUnavailable => ApiErrorCode::InputUnavailable,
            AppError::OutputExists => ApiErrorCode::OutputExists,
            AppError::InvalidInput(_) | AppError::Json(_) => ApiErrorCode::InvalidInput,
            AppError::Sync(message) => {
                let reason = if message.contains("insufficient_role") {
                    "insufficient_role"
                } else if message.contains("device_revoked") {
                    "device_revoked"
                } else if message == "sync_config_local_changed" {
                    "sync_config_local_changed"
                } else if message == "sync_config_conflict_changed" {
                    "sync_config_conflict_changed"
                } else if message.contains("sync_not_configured") {
                    "sync_not_configured"
                } else if message.contains("sync_workspace_not_attached") {
                    "sync_workspace_not_attached"
                } else if message.contains("missing encryption key") {
                    "missing_encryption_key"
                } else {
                    "sync_failed"
                };
                let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
                result.details = Some(ApiErrorDetails::SyncFailure {
                    reason: reason.into(),
                });
                return result;
            }
            AppError::Io(error) if error.kind() == std::io::ErrorKind::StorageFull => {
                ApiErrorCode::StorageFull
            }
            AppError::Io(_) => ApiErrorCode::IoFailed,
            AppError::Database(error) => database_code(&error),
        };
        Self::new(code, request_id)
    }

    fn from_core(error: pkms_core::Error, request_id: &str) -> Self {
        use pkms_core::Error;
        if let Error::InvalidInput(reason) = &error
            && (reason.starts_with("knowledge_task_")
                || reason.starts_with("task_template_")
                || *reason == "chat_evidence_invalid")
        {
            let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
            result.details = Some(ApiErrorDetails::InferenceFailure {
                reason: (*reason).into(),
            });
            return result;
        }
        if let Error::InvalidInput(
            reason @ ("tag name is already in use"
            | "tag is used by saved views"
            | "tag description exceeds bounds"),
        ) = &error
        {
            let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
            result.details = Some(ApiErrorDetails::TagFailure {
                reason: match *reason {
                    "tag name is already in use" => "duplicate_name",
                    "tag is used by saved views" => "view_in_use",
                    _ => "invalid_description",
                }
                .into(),
            });
            return result;
        }
        if let Error::InvalidInput(
            reason @ ("csl_required"
            | "csl_unreadable"
            | "csl_too_large"
            | "csl_not_utf8"
            | "csl_invalid_style"
            | "csl_invalid_identity"
            | "csl_invalid_parent"
            | "csl_builtin_id_reserved"
            | "csl_parent_missing"
            | "csl_parent_cycle"
            | "csl_parent_limit"
            | "csl_builtin_readonly"
            | "csl_style_in_use"),
        ) = &error
        {
            let mut result = Self::new(ApiErrorCode::InvalidInput, request_id);
            result.details = Some(ApiErrorDetails::CitationFailure {
                reason: (*reason).into(),
            });
            return result;
        }
        let code = match error {
            Error::InvalidInput(
                reason @ ("voice_audio_invalid"
                | "voice_audio_too_large"
                | "voice_duration_too_long"
                | "voice_title_invalid"
                | "voice_language_invalid"
                | "voice_device_label_invalid"
                | "voice_transcript_empty"
                | "voice_transcript_too_long"
                | "asr_settings_incomplete"
                | "asr_settings_invalid"
                | "asr_endpoint_invalid"
                | "asr_empty_transcript"
                | "asr_transcript_too_long"
                | "asr_already_running"
                | "asr_error_code_invalid"),
            ) => {
                return Self::from_app(AppError::InvalidInput(reason), request_id);
            }
            Error::InvalidInput("model_unsupported_output") => {
                return Self::from_app(
                    AppError::InvalidInput("model_unsupported_output"),
                    request_id,
                );
            }
            Error::QueryExpired => ApiErrorCode::QueryExpired,
            Error::RevisionConflict { expected, actual } => {
                let mut result = Self::new(ApiErrorCode::RevisionConflict, request_id);
                result.details = Some(ApiErrorDetails::RevisionConflict { expected, actual });
                return result;
            }
            Error::Conflict { .. } => ApiErrorCode::RevisionConflict,
            Error::MetadataConflict {
                object_id,
                expected,
                actual,
            } => {
                let mut result = Self::new(ApiErrorCode::MetadataConflict, request_id);
                result.details = Some(ApiErrorDetails::MetadataConflict {
                    object_id,
                    expected,
                    actual,
                });
                return result;
            }
            Error::InvalidInput(_) => ApiErrorCode::InvalidInput,
            Error::InvalidDocument(_) | Error::Json(_) => ApiErrorCode::InvalidDocument,
            Error::IdempotencyMismatch => ApiErrorCode::IdempotencyMismatch,
            Error::NotFound => ApiErrorCode::NotFound,
            Error::Trashed => ApiErrorCode::ObjectTrashed,
            Error::WorkspaceBusy => ApiErrorCode::StorageBusy,
            Error::UnsupportedDatabase => ApiErrorCode::UnsupportedDatabase,
            Error::MigrationHistoryMismatch { .. } => ApiErrorCode::MigrationFailed,
            Error::IntegrityFailure => ApiErrorCode::IntegrityFailed,
            Error::Io(error) if error.kind() == std::io::ErrorKind::StorageFull => {
                ApiErrorCode::StorageFull
            }
            Error::Io(_) => ApiErrorCode::IoFailed,
            Error::Database(error) => database_code(&error),
            Error::BackupTimeout => ApiErrorCode::IoFailed,
        };
        Self::new(code, request_id)
    }
}

fn database_code(error: &rusqlite::Error) -> ApiErrorCode {
    match error.sqlite_error_code() {
        Some(rusqlite::ErrorCode::DiskFull) => ApiErrorCode::StorageFull,
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
            ApiErrorCode::StorageBusy
        }
        Some(rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase) => {
            ApiErrorCode::IntegrityFailed
        }
        _ => ApiErrorCode::Internal,
    }
}

impl DesktopRequest {
    pub fn parse(value: serde_json::Value) -> std::result::Result<Self, ApiError> {
        let request_id = value
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .chars()
            .take(64)
            .collect::<String>();
        if value.to_string().len() > 8 * 1024 * 1024 {
            return Err(ApiError::new(ApiErrorCode::InvalidInput, &request_id));
        }
        let request: Self = serde_json::from_value(value)
            .map_err(|_| ApiError::new(ApiErrorCode::InvalidInput, &request_id))?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> std::result::Result<(), ApiError> {
        if self.api_version != 1 {
            return Err(ApiError::new(
                ApiErrorCode::UnsupportedApi,
                &self.request_id,
            ));
        }
        if uuid::Uuid::parse_str(&self.request_id).is_err() {
            return Err(ApiError::new(ApiErrorCode::InvalidInput, &self.request_id));
        }
        if let Some(identity) = &self.identity
            && (uuid::Uuid::parse_str(&identity.local_instance_id).is_err()
                || uuid::Uuid::parse_str(&identity.workspace_id).is_err())
        {
            return Err(ApiError::new(ApiErrorCode::InvalidInput, &self.request_id));
        }
        Ok(())
    }
}

/// The save-time embedding probe runs before a profile exists, so it is bounded
/// by its own 30 second timeout rather than by application shutdown.
static PROBE_NOT_CANCELLABLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Remote embedding and generation configuration is refused until the user has
/// acknowledged, for this workspace, that enabled text leaves the device.
fn require_remote_consent(library: &pkms_core::Library) -> Result<()> {
    crate::privacy::require_remote_consent(library)
}

impl AppService {
    /// Freeze a saved, live channel before touching its credentials or remote catalog.
    /// Native network operations recheck the same revision after releasing the service lock.
    pub fn generation_channel_at_version(
        &self,
        identity: &WorkspaceIdentity,
        channel_id: &str,
        row_version: u32,
    ) -> Result<pkms_core::GenerationChannel> {
        let channel = self.library(identity)?.generation_channel(channel_id)?;
        if channel.row_version != row_version {
            return Err(pkms_core::Error::MetadataConflict {
                object_id: channel.id,
                expected: row_version,
                actual: channel.row_version,
            }
            .into());
        }
        if channel.deleted {
            return Err(AppError::InvalidInput("generation_channel_deleted"));
        }
        Ok(channel)
    }

    /// Prepare a transcription under the service lock, then release the lock
    /// before the native adapter performs the slow network call.
    pub fn begin_voice_transcription(
        &mut self,
        identity: &WorkspaceIdentity,
        capture_id: &str,
        base_row_version: Option<u32>,
    ) -> Result<VoiceTranscription> {
        let library = self.library_mut(identity)?;
        let settings = library.asr_settings()?;
        if settings.endpoint.is_empty() || settings.model.is_empty() {
            return Err(AppError::InvalidInput("asr_not_configured"));
        }
        if crate::asr::is_remote(&settings) {
            crate::privacy::require_remote_consent(library)?;
        }
        let secret = crate::asr::load_secret(identity, &settings)?;
        let audio = library.voice_capture_audio(capture_id)?;
        let capture = library.begin_voice_transcription(capture_id, base_row_version)?;
        Ok(VoiceTranscription {
            job: AsrJob {
                settings,
                secret,
                audio,
                media_type: capture.audio_media_type,
                row_version: capture.row_version,
            },
            capture_id: capture_id.to_owned(),
        })
    }

    pub fn finish_voice_transcription(
        &mut self,
        identity: &WorkspaceIdentity,
        transcription: &VoiceTranscription,
        result: std::result::Result<AsrOutcome, AppError>,
    ) -> Result<pkms_core::VoiceCapture> {
        let library = self.library_mut(identity)?;
        let capture_id = transcription.capture_id();
        let row_version = transcription.row_version();
        match result {
            Ok(outcome) => Ok(library.finish_voice_transcription(
                capture_id,
                row_version,
                &outcome.text,
                &outcome.segments,
            )?),
            Err(AppError::InvalidInput(code)) => {
                Ok(library.fail_voice_transcription(capture_id, row_version, code)?)
            }
            Err(_) => {
                Ok(library.fail_voice_transcription(capture_id, row_version, "asr_failed")?)
            }
        }
    }

    pub fn asr_probe(&self, identity: &WorkspaceIdentity) -> Result<AsrProbe> {
        let settings = self.library(identity)?.asr_settings()?;
        if settings.endpoint.is_empty() || settings.model.is_empty() {
            return Err(AppError::InvalidInput("asr_not_configured"));
        }
        let secret = crate::asr::load_secret(identity, &settings)?;
        Ok(AsrProbe { settings, secret })
    }

    pub fn voice_summary_input(
        &self,
        identity: &WorkspaceIdentity,
        capture_id: &str,
    ) -> Result<(pkms_core::GenerationProfile, String)> {
        let library = self.library(identity)?;
        let capture = library.voice_capture(capture_id)?;
        let profile = library
            .generation_settings()?
            .profile
            .ok_or(AppError::InvalidInput("model_profile_missing"))?;
        crate::privacy::require_generation_consent(library, &profile)?;
        Ok((profile, capture.transcript))
    }

    pub fn dispatch(
        &mut self,
        request: DesktopRequest,
    ) -> std::result::Result<DesktopReply, ApiError> {
        request.validate()?;
        let request_id = request.request_id;
        let result = self
            .dispatch_command(request.identity.as_ref(), request.command)
            .map_err(|error| ApiError::from_app(error, &request_id))?;
        Ok(DesktopReply {
            request_id,
            data: result,
        })
    }

    fn dispatch_command(
        &mut self,
        identity: Option<&WorkspaceIdentity>,
        command: DesktopCommand,
    ) -> Result<DesktopData> {
        let scope = || identity.ok_or(AppError::InstanceMismatch);
        Ok(match command {
            DesktopCommand::WorkspaceStatistics => {
                DesktopData::WorkspaceStatistics(self.library(scope()?)?.workspace_statistics()?)
            }
            DesktopCommand::DiagnosticsPreview { categories } => {
                DesktopData::DiagnosticsPreview(self.diagnostics_preview(scope()?, categories)?)
            }
            DesktopCommand::DiagnosticsExport {
                command_id,
                token,
                categories,
                fingerprint,
            } => DesktopData::JobAccepted(self.export_diagnostics(
                scope()?,
                &command_id,
                &token,
                categories,
                &fingerprint,
            )?),
            DesktopCommand::ReviewList(request) => {
                DesktopData::ReviewPage(self.library(scope()?)?.review_list(&request)?)
            }
            DesktopCommand::ReviewCounts { collection_id } => DesktopData::ReviewCounts(
                self.library(scope()?)?
                    .review_counts(collection_id.as_deref())?,
            ),
            DesktopCommand::ReviewGet { entry_id } => {
                DesktopData::ReviewItem(self.library(scope()?)?.review_entry(&entry_id)?)
            }
            DesktopCommand::ReviewHistory(request) => {
                DesktopData::ReviewEventPage(self.library(scope()?)?.review_history(&request)?)
            }
            DesktopCommand::ReviewAdd {
                command_id,
                request,
            } => DesktopData::ReviewMutation(
                self.library_mut(scope()?)?
                    .add_review(&command_id, request)?,
            ),
            DesktopCommand::ReviewComplete {
                command_id,
                request,
            } => DesktopData::ReviewMutation(
                self.library_mut(scope()?)?
                    .complete_review(&command_id, request)?,
            ),
            DesktopCommand::ReviewSnooze {
                command_id,
                request,
            } => DesktopData::ReviewMutation(
                self.library_mut(scope()?)?
                    .snooze_review(&command_id, request)?,
            ),
            DesktopCommand::ReviewRemove {
                command_id,
                request,
            } => DesktopData::ReviewMutation(
                self.library_mut(scope()?)?
                    .remove_review(&command_id, request)?,
            ),
            DesktopCommand::AutomationChangeSetsPage(request) => {
                DesktopData::ChangeSetPage(self.library(scope()?)?.change_sets_page(request)?)
            }
            DesktopCommand::AutomationChangeSetGet { change_id } => {
                DesktopData::ChangeSet(self.library(scope()?)?.change_set(&change_id)?)
            }
            DesktopCommand::AutomationCreateChangeSet {
                command_id,
                request,
            } => DesktopData::ChangeSetCommit(
                self.library_mut(scope()?)?
                    .create_change_set(&command_id, request)?,
            ),
            DesktopCommand::AutomationApplyItem {
                command_id,
                request,
            } => DesktopData::ChangeItemCommit(
                self.library_mut(scope()?)?
                    .apply_change_item(&command_id, request)?,
            ),
            DesktopCommand::AutomationSkipItem {
                command_id,
                request,
            } => DesktopData::ChangeItemCommit(
                self.library_mut(scope()?)?
                    .skip_change_item(&command_id, request)?,
            ),
            DesktopCommand::AutomationRejectChangeSet {
                command_id,
                request,
            } => DesktopData::ChangeSetCommit(
                self.library_mut(scope()?)?
                    .reject_change_set(&command_id, request)?,
            ),
            DesktopCommand::AutomationExpire => {
                self.library_mut(scope()?)?
                    .expire_change_sets_before(chrono::Utc::now() - chrono::Duration::days(7))?;
                DesktopData::Done
            }
            DesktopCommand::AutomationStaleReport { window_days, limit } => {
                DesktopData::StaleSourceReport(
                    self.library(scope()?)?
                        .stale_source_report(window_days, limit)?,
                )
            }
            DesktopCommand::AuditPage(request) => {
                DesktopData::AuditPage(self.library(scope()?)?.audit_page(request)?)
            }
            DesktopCommand::AuditUndo {
                command_id,
                request,
            } => DesktopData::AuditUndoCommit(
                self.library_mut(scope()?)?
                    .undo_audit_event(&command_id, request)?,
            ),
            DesktopCommand::ResearchCreate {
                command_id,
                request,
            } => DesktopData::ResearchCommit(
                self.library_mut(scope()?)?
                    .create_research_task(&command_id, request)?,
            ),
            DesktopCommand::ResearchGet { task_id } => {
                DesktopData::ResearchTask(self.library(scope()?)?.research_task(&task_id)?)
            }
            DesktopCommand::ResearchPage(request) => {
                DesktopData::ResearchPage(self.library(scope()?)?.research_page(request)?)
            }
            DesktopCommand::ResearchAdvance {
                command_id,
                request,
            } => DesktopData::ResearchCommit(
                self.library_mut(scope()?)?
                    .advance_research(&command_id, request)?,
            ),
            DesktopCommand::ResearchCancel {
                command_id,
                request,
            } => DesktopData::ResearchCommit(
                self.library_mut(scope()?)?
                    .cancel_research(&command_id, request)?,
            ),
            DesktopCommand::ResearchRunExternal {
                command_id,
                task_id,
                base_row_version,
            } => DesktopData::ResearchCommit(self.run_external_research(
                scope()?,
                &command_id,
                &task_id,
                base_row_version,
            )?),
            DesktopCommand::ReviewInsightsGet { limit } => {
                DesktopData::ReviewInsights(self.library(scope()?)?.review_insights(limit)?)
            }
            DesktopCommand::ReviewInsightSettingsGet => DesktopData::ReviewInsightSettings(
                self.library(scope()?)?.review_insight_settings()?,
            ),
            DesktopCommand::ReviewInsightSettingsSet {
                command_id,
                request,
            } => DesktopData::ReviewSettingsCommit(
                self.library_mut(scope()?)?
                    .set_review_insight_settings(&command_id, request)?,
            ),
            DesktopCommand::AppState => {
                let active = self.active_workspace();
                let workspace_metadata = match &active {
                    Some(entry) => Some(self.library(&entry.identity)?.workspace_metadata()?),
                    None => None,
                };
                DesktopData::AppState(DesktopAppState {
                    active,
                    preferences: self.preferences()?,
                    workspace_metadata,
                    workspaces: self.workspaces()?,
                    parse_components: crate::parse_component_issues(),
                })
            }
            DesktopCommand::Capabilities => DesktopData::Capabilities(AppCapabilities {
                platform: std::env::consts::OS.into(),
                document_schemas: pkms_core::SUPPORTED_DOCUMENT_SCHEMAS.to_vec(),
                image_formats: vec!["png".into(), "jpeg".into(), "webp".into()],
            }),
            DesktopCommand::WorkspacesList => DesktopData::Workspaces(self.workspaces()?),
            DesktopCommand::WorkspaceSelect { target } => {
                if self.active_workspace().map(|entry| entry.identity).as_ref() != identity {
                    return Err(AppError::InstanceMismatch);
                }
                let entry = self
                    .workspaces()?
                    .into_iter()
                    .find(|entry| entry.identity == target)
                    .ok_or(AppError::NotFound)?;
                DesktopData::ResourceGrant(
                    self.grant_path(GrantPurpose::OpenWorkspace, entry.path)?,
                )
            }
            DesktopCommand::WorkspaceForget => {
                self.forget_workspace(scope()?)?;
                DesktopData::Done
            }
            DesktopCommand::WorkspaceClose => {
                self.library(scope()?)?;
                self.close_workspace();
                DesktopData::Done
            }
            DesktopCommand::PreferencesGet => DesktopData::Preferences(self.preferences()?),
            DesktopCommand::PreferencesUpdate {
                base_state_version,
                preferences,
            } => {
                DesktopData::Preferences(self.update_preferences(base_state_version, preferences)?)
            }
            DesktopCommand::WorkspaceMetadataGet => {
                DesktopData::WorkspaceMetadata(self.library(scope()?)?.workspace_metadata()?)
            }
            DesktopCommand::WorkspaceMetadataUpdate { request } => DesktopData::WorkspaceMetadata(
                self.library_mut(scope()?)?
                    .update_workspace_metadata(request)?,
            ),
            DesktopCommand::VoiceCaptures { trashed } => {
                DesktopData::VoiceCaptures(self.library(scope()?)?.voice_captures(trashed)?)
            }
            DesktopCommand::VoiceBrowse(request) => {
                DesktopData::VoiceBrowse(self.library(scope()?)?.voice_browse(request)?)
            }
            DesktopCommand::VoiceCaptureGet { capture_id } => {
                DesktopData::VoiceCapture(self.library(scope()?)?.voice_capture(&capture_id)?)
            }
            DesktopCommand::VoiceSyncTasks => {
                DesktopData::VoiceSyncTasks(self.library(scope()?)?.voice_sync_tasks()?)
            }
            DesktopCommand::VoiceCaptureCreate {
                command_id,
                request,
                audio_base64,
            } => {
                // Bound the encoded size before allocating the decoded buffer.
                if audio_base64.len() > (pkms_core::VOICE_MAX_BYTES / 3 + 1) * 4 {
                    return Err(AppError::InvalidInput("voice_audio_too_large"));
                }
                let bytes = BASE64
                    .decode(audio_base64.as_bytes())
                    .map_err(|_| AppError::InvalidInput("voice_audio_invalid"))?;
                DesktopData::VoiceCapture(self.library_mut(scope()?)?.create_voice_capture(
                    &command_id,
                    request,
                    &bytes,
                )?)
            }
            DesktopCommand::VoiceCaptureUpdate { request } => DesktopData::VoiceCapture(
                self.library_mut(scope()?)?.update_voice_capture(request)?,
            ),
            DesktopCommand::VoiceCaptureTrash { request } => {
                DesktopData::VoiceCapture(self.library_mut(scope()?)?.trash_voice_capture(request)?)
            }
            DesktopCommand::VoiceCaptureSaveNote {
                command_id,
                request,
            } => DesktopData::VoiceCapture(
                self.library_mut(scope()?)?
                    .save_voice_capture_note(&command_id, request)?,
            ),
            DesktopCommand::VoiceCaptureTranscribe { .. }
            | DesktopCommand::VoiceCaptureSummarize { .. }
            | DesktopCommand::VoiceSyncImport
            | DesktopCommand::AsrSettingsTest => {
                return Err(AppError::InvalidInput(
                    "network_command_requires_native_adapter",
                ));
            }
            DesktopCommand::AsrSettingsGet => {
                let identity = scope()?;
                let settings = self.library(identity)?.asr_settings()?;
                let configured = crate::asr::credential_configured(identity, &settings)?;
                DesktopData::AsrSettings(pkms_core::AsrSettingsState {
                    settings,
                    credential_configured: configured,
                })
            }
            DesktopCommand::AsrSettingsSave(save) => {
                let identity = scope()?;
                let settings = self
                    .library_mut(identity)?
                    .update_asr_settings(save.request)?;
                if save.clear_credential || settings.endpoint.is_empty() {
                    crate::asr::clear_secret(identity, &settings)?;
                } else if let Some(credential) = save.credential {
                    crate::asr::store_secret(identity, &settings, &credential.0)?;
                }
                let configured = crate::asr::credential_configured(identity, &settings)?;
                DesktopData::AsrSettings(pkms_core::AsrSettingsState {
                    settings,
                    credential_configured: configured,
                })
            }
            DesktopCommand::SessionGet => DesktopData::Session(self.session(scope()?)?),
            DesktopCommand::SessionSave {
                base_state_version,
                layout,
            } => DesktopData::Session(self.save_session(scope()?, base_state_version, layout)?),
            DesktopCommand::RecentObjectsList => {
                DesktopData::RecentObjects(self.recent_objects(scope()?)?)
            }
            DesktopCommand::RecentObjectsTouch { object_id } => {
                self.touch_recent_object(scope()?, &object_id)?;
                DesktopData::Done
            }
            DesktopCommand::WorkspaceCreate { command_id, token } => {
                DesktopData::Bootstrap(self.accept_workspace_operation(
                    &command_id,
                    OperationKind::CreateWorkspace,
                    &token,
                )?)
            }
            DesktopCommand::WorkspaceOpen { command_id, token } => DesktopData::Bootstrap(
                self.accept_workspace_operation(&command_id, OperationKind::OpenWorkspace, &token)?,
            ),
            DesktopCommand::BackupInspect { token } => {
                DesktopData::BackupInspection(self.inspect_selected_backup(&token)?)
            }
            DesktopCommand::BackupVerify { command_id, token } => {
                DesktopData::Bootstrap(self.accept_backup_operation(
                    &command_id,
                    OperationKind::VerifyBackup,
                    &token,
                    None,
                )?)
            }
            DesktopCommand::BackupRestore {
                command_id,
                token,
                output_token,
            } => DesktopData::Bootstrap(self.accept_backup_operation(
                &command_id,
                OperationKind::RestoreBackup,
                &token,
                Some(&output_token),
            )?),
            DesktopCommand::BackupCreate { command_id, token } => {
                DesktopData::JobAccepted(self.accept_output_job(
                    scope()?,
                    &command_id,
                    &token,
                    pkms_core::PortableJobInput::BackupCreate,
                )?)
            }
            DesktopCommand::HandoffExport { token } => {
                let identity = scope()?;
                let destination = self.resolve_grant(&token, &GrantPurpose::HandoffOutput)?;
                let handoff = self
                    .library(identity)?
                    .export_resume_handoff(&[], &std::collections::HashMap::new())?;
                let bytes = serde_json::to_vec_pretty(&handoff)?;
                std::fs::write(destination, bytes)?;
                DesktopData::HandoffExport(HandoffExportSummary {
                    workspace_id: handoff.workspace_id,
                    objects: handoff.objects.len(),
                })
            }
            DesktopCommand::HandoffImport { token } => {
                let identity = scope()?;
                let source = self.resolve_grant(&token, &GrantPurpose::HandoffInput)?;
                const MAX_HANDOFF_BYTES: u64 = 16 * 1024 * 1024;
                let mut bytes = Vec::new();
                use std::io::Read;
                std::fs::File::open(source)?
                    .take(MAX_HANDOFF_BYTES + 1)
                    .read_to_end(&mut bytes)?;
                if bytes.len() as u64 > MAX_HANDOFF_BYTES {
                    return Err(AppError::InvalidInput("resume handoff is too large"));
                }
                let handoff = serde_json::from_slice(&bytes)?;
                let imported = self
                    .library_mut(identity)?
                    .import_resume_handoff(&handoff)?;
                DesktopData::HandoffImport(HandoffImportSummary {
                    imported: imported.imported.into_iter().map(|note| note.id).collect(),
                    skipped_existing: imported.skipped_existing,
                })
            }
            DesktopCommand::SyncStatus => DesktopData::SyncStatus(self.sync_status()?),
            DesktopCommand::SyncConfigure {
                endpoint,
                display_name,
            } => DesktopData::SyncProfile(self.sync_configure(&endpoint, &display_name)?),
            DesktopCommand::SyncPair { invite_id, code } => {
                DesktopData::SyncProfile(self.sync_pair(&invite_id, &code)?)
            }
            DesktopCommand::SyncOwnerProof => DesktopData::SyncOwnerProof(self.sync_owner_proof()?),
            DesktopCommand::SyncClaimDevice { device_id } => {
                DesktopData::SyncProfile(self.sync_claim_device(&device_id)?)
            }
            DesktopCommand::SyncCreateInvite { role, member_name } => {
                DesktopData::SyncInvite(self.sync_create_invite(&member_name, role)?)
            }
            DesktopCommand::SyncCreateDeviceInvite { member_id, role } => {
                DesktopData::SyncInvite(self.sync_create_device_invite(&member_id, role)?)
            }
            DesktopCommand::SyncSetDeviceRole {
                target_device_id,
                role,
            } => DesktopData::SyncDevice(self.sync_set_device_role(&target_device_id, role)?),
            DesktopCommand::SyncMembers => DesktopData::SyncMembers(self.sync_members()?),
            DesktopCommand::SyncAudit { limit } => {
                DesktopData::SyncAudit(self.sync_audit(limit as usize)?)
            }
            DesktopCommand::SyncSetMemberRole { member_id, role } => {
                DesktopData::SyncMember(self.sync_set_member_role(&member_id, role)?)
            }
            DesktopCommand::SyncRemoveMember { member_id, reason } => {
                DesktopData::SyncMemberRemoval(self.sync_remove_member(&member_id, &reason)?)
            }
            DesktopCommand::SyncDevices => DesktopData::SyncDevices(self.sync_devices()?),
            DesktopCommand::SyncRevokeDevice {
                target_device_id,
                reason,
            } => DesktopData::SyncDevice(self.sync_revoke_device(&target_device_id, &reason)?),
            DesktopCommand::SyncAttachWorkspace { workspace_id } => {
                DesktopData::SyncProfile(self.sync_attach_workspace(&workspace_id)?)
            }
            DesktopCommand::SyncEnsureWorkspaceKey { workspace_id } => {
                DesktopData::SyncProfile(self.sync_ensure_workspace_key(&workspace_id)?)
            }
            DesktopCommand::SyncGrantWorkspaceKey {
                workspace_id,
                target_device_id,
            } => {
                self.sync_grant_workspace_key(&workspace_id, &target_device_id)?;
                DesktopData::Done
            }
            DesktopCommand::SyncPushNote { note_id } => {
                DesktopData::SyncPush(self.sync_push_note(scope()?, &note_id)?)
            }
            DesktopCommand::SyncAutoPushNote { note_id } => {
                DesktopData::SyncAutoPush(self.sync_auto_push_note(scope()?, &note_id)?)
            }
            DesktopCommand::SyncPushAll => DesktopData::SyncPushAll(self.sync_push_all(scope()?)?),
            DesktopCommand::SyncAutoPushTags => {
                DesktopData::SyncTagPush(self.sync_auto_push_tags(scope()?)?)
            }
            DesktopCommand::SyncTagPage(request) => {
                DesktopData::SyncTagPage(self.sync_tag_page(scope()?, request)?)
            }
            DesktopCommand::SyncTagRefresh { kind, entity_id } => {
                DesktopData::SyncTagSnapshot(self.sync_tag_refresh(scope()?, kind, &entity_id)?)
            }
            DesktopCommand::SyncTagKeepLocal(decision) => {
                DesktopData::SyncTagSnapshot(self.sync_tag_keep_local(scope()?, decision)?)
            }
            DesktopCommand::SyncTagKeepRemote(decision) => {
                DesktopData::SyncTagSnapshot(self.sync_tag_keep_remote(scope()?, decision)?)
            }
            DesktopCommand::SyncPull => DesktopData::SyncPull(self.sync_pull(scope()?)?),
            DesktopCommand::SyncConfigConflicts => {
                DesktopData::SyncConfigConflicts(self.sync_config_conflicts(scope()?)?)
            }
            DesktopCommand::SyncConfigKeepLocal {
                object_id,
                remote_version,
                local_state_token,
            } => DesktopData::SyncConfigResolution(self.sync_config_keep_local(
                scope()?,
                &object_id,
                remote_version,
                &local_state_token,
            )?),
            DesktopCommand::SyncConfigKeepRemote {
                object_id,
                remote_version,
                local_state_token,
            } => DesktopData::SyncConfigResolution(self.sync_config_keep_remote(
                scope()?,
                &object_id,
                remote_version,
                &local_state_token,
            )?),
            DesktopCommand::SyncWatch { timeout_ms } => {
                DesktopData::SyncWatch(self.sync_wait_for_change(scope()?, u64::from(timeout_ms))?)
            }
            DesktopCommand::SyncAutoPushWorkspaceMetadata => {
                DesktopData::SyncAutoPush(self.sync_auto_push_workspace_metadata(scope()?)?)
            }
            DesktopCommand::SyncResolveKeepLocal { conflict } => {
                DesktopData::SyncPush(self.sync_resolve_keep_local(scope()?, &conflict)?)
            }
            DesktopCommand::SyncResolveKeepRemote { conflict } => {
                self.sync_resolve_keep_remote(scope()?, &conflict)?;
                DesktopData::Done
            }
            DesktopCommand::SyncResolveSaveAsNew { conflict } => {
                DesktopData::SyncObjectId(self.sync_resolve_save_as_new(scope()?, &conflict)?)
            }
            DesktopCommand::SyncDeleteNote { note_id } => {
                DesktopData::SyncPush(self.sync_delete_note(scope()?, &note_id)?)
            }
            DesktopCommand::SyncRestoreNote { note_id } => {
                DesktopData::SyncPush(self.sync_restore_note(scope()?, &note_id)?)
            }
            DesktopCommand::SyncTrash => DesktopData::SyncTrash(self.sync_trash(identity)?),
            DesktopCommand::NoteExport {
                command_id,
                token,
                note_id,
                revision_id,
                format,
                citation_style_id,
            } => DesktopData::JobAccepted(self.accept_output_job(
                scope()?,
                &command_id,
                &token,
                pkms_core::PortableJobInput::ExportNote {
                    note_id,
                    revision_id,
                    format,
                    citation_style_id,
                },
            )?),
            DesktopCommand::SourceExport {
                command_id,
                token,
                source_id,
                version_id,
            } => DesktopData::JobAccepted(self.accept_output_job(
                scope()?,
                &command_id,
                &token,
                pkms_core::PortableJobInput::ExportSource {
                    source_id,
                    version_id,
                },
            )?),
            DesktopCommand::JobsList { limit } => {
                DesktopData::Jobs(self.library(scope()?)?.portable_jobs(limit)?)
            }
            DesktopCommand::JobsPage { limit, cursor } => {
                DesktopData::JobsPage(self.library(scope()?)?.portable_jobs_page(limit, cursor)?)
            }
            DesktopCommand::JobGet { job_id } => {
                DesktopData::Job(self.library(scope()?)?.portable_job(&job_id)?)
            }
            DesktopCommand::JobCancel { job_id } => {
                DesktopData::Job(self.library(scope()?)?.cancel_portable_job(&job_id)?)
            }
            DesktopCommand::OperationsList { limit } => {
                DesktopData::Operations(self.operations(limit)?)
            }
            DesktopCommand::OperationsPage { limit, cursor } => {
                DesktopData::OperationsPage(self.operations_page(limit, cursor)?)
            }
            DesktopCommand::OperationGet { operation_id } => {
                DesktopData::Operation(self.operation(&operation_id)?)
            }
            DesktopCommand::OperationCancel { operation_id } => {
                DesktopData::Operation(self.cancel_operation(&operation_id)?)
            }
            DesktopCommand::WorkspaceInfo => {
                DesktopData::WorkspaceInfo(self.library(scope()?)?.info()?)
            }
            DesktopCommand::ObjectsMetadataGet { object_id } => {
                DesktopData::ObjectMetadata(self.library(scope()?)?.object_metadata(&object_id)?)
            }
            DesktopCommand::TrashList(request) => {
                DesktopData::Trash(self.library(scope()?)?.trash_page(request)?)
            }
            DesktopCommand::TrashPreview(request) => {
                DesktopData::TrashPreview(self.library(scope()?)?.preview_trash_delete(request)?)
            }
            DesktopCommand::TrashDelete {
                command_id,
                request,
            } => DesktopData::TrashCommit(self.delete_trash(scope()?, &command_id, request)?),
            DesktopCommand::ObjectsSetTrashed {
                command_id,
                request,
            } => DesktopData::MetadataCommit(
                self.library_mut(scope()?)?
                    .set_object_trashed(&command_id, request)?,
            ),
            DesktopCommand::ObjectsSetArchived {
                command_id,
                request,
            } => DesktopData::MetadataCommit(
                self.library_mut(scope()?)?
                    .set_object_archived(&command_id, request)?,
            ),
            DesktopCommand::ObjectsSetSearchEnabled {
                command_id,
                request,
            } => DesktopData::MetadataCommit(
                self.library_mut(scope()?)?
                    .set_object_search_enabled(&command_id, request)?,
            ),
            DesktopCommand::CollectionsGet { collection_id } => {
                DesktopData::Collection(self.library(scope()?)?.collection_get(&collection_id)?)
            }
            DesktopCommand::CollectionsList(query) => {
                DesktopData::Collections(self.library(scope()?)?.collections_page(query)?)
            }
            DesktopCommand::CollectionsMembers(query) => DesktopData::CollectionMembers(
                self.library(scope()?)?.collection_members_page(query)?,
            ),
            DesktopCommand::CollectionsCreate {
                command_id,
                request,
            } => DesktopData::CollectionCommit(
                self.library_mut(scope()?)?
                    .collection_create(&command_id, request)?,
            ),
            DesktopCommand::CollectionsUpdate {
                command_id,
                request,
            } => DesktopData::CollectionCommit(
                self.library_mut(scope()?)?
                    .collection_update(&command_id, request)?,
            ),
            DesktopCommand::CollectionsAddMembers {
                command_id,
                request,
            } => DesktopData::CollectionCommit(
                self.library_mut(scope()?)?
                    .collection_add_members(&command_id, request)?,
            ),
            DesktopCommand::CollectionsRemoveMembers {
                command_id,
                request,
            } => DesktopData::CollectionCommit(
                self.library_mut(scope()?)?
                    .collection_remove_members(&command_id, request)?,
            ),
            DesktopCommand::CollectionsReorder {
                command_id,
                request,
            } => DesktopData::CollectionCommit(
                self.library_mut(scope()?)?
                    .collection_reorder(&command_id, request)?,
            ),
            DesktopCommand::ViewsGet { view_id } => {
                DesktopData::View(self.library(scope()?)?.view_get(&view_id)?)
            }
            DesktopCommand::ViewsList(query) => {
                DesktopData::Views(self.library(scope()?)?.views_page(query)?)
            }
            DesktopCommand::ViewsRun {
                view_id,
                limit,
                cursor,
            } => DesktopData::ViewRun(self.library(scope()?)?.view_run(&view_id, limit, cursor)?),
            DesktopCommand::ViewsSave {
                command_id,
                request,
            } => DesktopData::ViewCommit(
                self.library_mut(scope()?)?
                    .view_save(&command_id, request)?,
            ),
            DesktopCommand::ViewsRemove {
                command_id,
                request,
            } => DesktopData::ViewCommit(
                self.library_mut(scope()?)?
                    .view_remove(&command_id, request)?,
            ),
            DesktopCommand::CanvasesGet { canvas_id } => {
                DesktopData::Canvas(self.library(scope()?)?.canvas_get(&canvas_id)?)
            }
            DesktopCommand::CanvasesList(query) => {
                DesktopData::Canvases(self.library(scope()?)?.canvases_page(query)?)
            }
            DesktopCommand::CanvasesSave {
                command_id,
                request,
            } => DesktopData::CanvasCommit(
                self.library_mut(scope()?)?
                    .canvas_save(&command_id, request)?,
            ),
            DesktopCommand::CanvasesRemove {
                command_id,
                request,
            } => DesktopData::CanvasCommit(
                self.library_mut(scope()?)?
                    .canvas_remove(&command_id, request)?,
            ),
            DesktopCommand::TaggedObjects(query) => {
                DesktopData::TaggedObjectsPage(self.library(scope()?)?.tagged_objects(query)?)
            }
            DesktopCommand::TagsBrowse(query) => {
                DesktopData::TagsBrowsePage(self.library(scope()?)?.tags_browse(query)?)
            }
            DesktopCommand::TagsGet { tag_id } => {
                DesktopData::TagDetails(self.library(scope()?)?.tag_details(&tag_id)?)
            }
            DesktopCommand::TagsCreate {
                command_id,
                request,
            } => DesktopData::TagManagementCommit(
                self.library_mut(scope()?)?
                    .tag_create(&command_id, request)?,
            ),
            DesktopCommand::TagsUpdate {
                command_id,
                request,
            } => DesktopData::TagManagementCommit(
                self.library_mut(scope()?)?
                    .tag_update(&command_id, request)?,
            ),
            DesktopCommand::TagsDelete {
                command_id,
                request,
            } => DesktopData::TagManagementCommit(
                self.library_mut(scope()?)?
                    .tag_delete(&command_id, request)?,
            ),
            DesktopCommand::TagsList(query) => {
                DesktopData::Tags(self.library(scope()?)?.tags_page(query)?)
            }
            DesktopCommand::TagsAssign {
                command_id,
                request,
            } => DesktopData::MetadataCommit(
                self.library_mut(scope()?)?
                    .assign_tag(&command_id, request)?,
            ),
            DesktopCommand::TagsRemove {
                command_id,
                request,
            } => DesktopData::MetadataCommit(
                self.library_mut(scope()?)?
                    .remove_tag(&command_id, request)?,
            ),
            DesktopCommand::ObjectsSetInboxState {
                command_id,
                request,
            } => DesktopData::MetadataCommit(
                self.library_mut(scope()?)?
                    .set_inbox_state(&command_id, request)?,
            ),
            DesktopCommand::KnowledgeObjectsList(query) => DesktopData::KnowledgeObjects(
                self.library(scope()?)?.knowledge_objects_page(query)?,
            ),
            DesktopCommand::KnowledgeBrowse(query) => {
                DesktopData::KnowledgeBrowse(self.library(scope()?)?.knowledge_browse(query)?)
            }
            DesktopCommand::NotesList(query) => {
                DesktopData::Notes(self.library(scope()?)?.notes_page(query)?)
            }
            DesktopCommand::SearchQuery(query) => {
                let library = self.library(scope()?)?;
                if query.mode != pkms_core::SearchMode::Lexical {
                    return Err(AppError::InvalidInput(
                        "semantic search requires native inference adapter",
                    ));
                }
                DesktopData::SearchResults(library.keyword_search(&query)?)
            }
            DesktopCommand::EmbeddingProfileGet => {
                DesktopData::EmbeddingProfile(self.library(scope()?)?.active_embedding_profile()?)
            }
            DesktopCommand::EmbeddingProfileRegister { command_id } => {
                DesktopData::EmbeddingProfileMutation(
                    self.library_mut(scope()?)?.register_embedding_profile(
                        &command_id,
                        pkms_inference::profile::profile_spec(),
                    )?,
                )
            }
            DesktopCommand::EmbeddingRemoteConfigure {
                command_id,
                provider,
                endpoint,
                model,
                output_dimensions,
            } => {
                let identity = scope()?;
                // Semantic text may leave the device only after an explicit,
                // library-level acknowledgement.
                require_remote_consent(self.library(identity)?)?;
                // The library's chunk cap and overlap are part of the profile
                // identity, so saving them re-registers this profile and rebuilds
                // the index while keeping the previous generation for rollback.
                let params = self.library(identity)?.chunk_settings()?.params;
                let mut remote = pkms_inference::remote_chunks::RemoteProfile {
                    provider,
                    endpoint: endpoint.trim_end_matches('/').to_owned(),
                    model: model.trim().to_owned(),
                    dimensions: 0,
                    output_dimensions,
                    chunk_tokens: params.chunk_tokens,
                    overlap_tokens: pkms_inference::remote_chunks::overlap_for(
                        params.chunk_tokens,
                        params.overlap_percent,
                    ),
                };
                // A profile is only registered with the size the provider really
                // returns: assuming a dimension would silently mix vector spaces,
                // and the model in the response is the only authority for it.
                let probe =
                    crate::inference_embeddings::probe(identity, &remote, &PROBE_NOT_CANCELLABLE)?;
                if output_dimensions.is_some_and(|value| value != probe.dimensions) {
                    return Err(AppError::InvalidInput("embedding_dimension_mismatch"));
                }
                remote.dimensions = probe.dimensions;
                let model = remote.model.clone();
                let registered = self.library_mut(identity)?.register_embedding_profile(
                    &command_id,
                    pkms_inference::remote_chunks::profile_spec(&remote),
                )?;
                crate::inference_queries::record_embedding_probe(
                    self.library_mut(identity)?,
                    &model,
                    probe.prompt_tokens,
                );
                DesktopData::EmbeddingProfileMutation(registered)
            }
            DesktopCommand::EmbeddingCredentialSet {
                provider,
                endpoint,
                model,
                api_key,
            } => {
                let identity = scope()?;
                self.library(identity)?;
                let binding = crate::inference_embeddings::binding(
                    identity,
                    provider,
                    &endpoint,
                    model.trim(),
                )?;
                let secret = crate::inference_credentials::Secret::new(api_key.0)
                    .map_err(crate::inference_remote::credential_error)?;
                crate::inference_credentials::store(&binding, &secret)
                    .map_err(crate::inference_remote::credential_error)?;
                DesktopData::Done
            }
            DesktopCommand::EmbeddingCredentialStatus {
                provider,
                endpoint,
                model,
            } => {
                let identity = scope()?;
                self.library(identity)?;
                let binding = crate::inference_embeddings::binding(
                    identity,
                    provider,
                    &endpoint,
                    model.trim(),
                )?;
                DesktopData::GenerationCredentialStatus(GenerationCredentialStatus {
                    configured: crate::inference_credentials::exists(&binding)
                        .map_err(crate::inference_remote::credential_error)?,
                })
            }
            DesktopCommand::EmbeddingCredentialDelete {
                provider,
                endpoint,
                model,
            } => {
                let identity = scope()?;
                self.library(identity)?;
                let binding = crate::inference_embeddings::binding(
                    identity,
                    provider,
                    &endpoint,
                    model.trim(),
                )?;
                crate::inference_credentials::delete(&binding)
                    .map_err(crate::inference_remote::credential_error)?;
                DesktopData::Done
            }
            DesktopCommand::IndexStatus => {
                DesktopData::IndexStatus(self.library(scope()?)?.index_status()?)
            }
            DesktopCommand::IndexRebuildStart {
                command_id,
                request,
            } => DesktopData::IndexRebuildMutation(
                self.library_mut(scope()?)?
                    .accept_index_rebuild(&command_id, request)?,
            ),
            DesktopCommand::IndexRebuildGet { job_id } => {
                DesktopData::IndexRebuild(self.library(scope()?)?.index_rebuild_job(&job_id)?)
            }
            DesktopCommand::IndexRebuildList { after_id, limit } => DesktopData::IndexRebuilds(
                self.library(scope()?)?
                    .index_rebuild_jobs_page(after_id.as_deref(), limit)?,
            ),
            DesktopCommand::IndexRebuildCancel { command_id, job_id } => {
                DesktopData::IndexRebuildMutation(
                    self.library_mut(scope()?)?
                        .cancel_index_rebuild(&command_id, &job_id)?,
                )
            }
            DesktopCommand::GenerationSettingsGet => {
                DesktopData::GenerationSettings(self.library(scope()?)?.generation_settings()?)
            }
            DesktopCommand::RetrievalSettingsGet => {
                DesktopData::RetrievalSettings(self.library(scope()?)?.retrieval_settings()?)
            }
            DesktopCommand::CleaningSettingsGet => {
                DesktopData::CleaningSettings(self.library(scope()?)?.cleaning_settings()?)
            }
            DesktopCommand::ChunkSettingsGet => {
                DesktopData::ChunkSettings(self.library(scope()?)?.chunk_settings()?)
            }
            DesktopCommand::EmbeddingProfilePrevious => {
                DesktopData::EmbeddingProfile(self.library(scope()?)?.previous_embedding_profile()?)
            }
            DesktopCommand::GenerationChannels => {
                DesktopData::GenerationChannels(self.library(scope()?)?.generation_channels()?)
            }
            DesktopCommand::GenerationChannelSave {
                command_id,
                request,
            } => {
                // Channels are remote by construction (validation rejects local
                // providers), so every channel save ends up sending text away.
                require_remote_consent(self.library(scope()?)?)?;
                DesktopData::GenerationChannelMutation(
                    self.library_mut(scope()?)?
                        .save_generation_channel(&command_id, request)?,
                )
            }
            DesktopCommand::GenerationChannelDelete {
                command_id,
                request,
            } => DesktopData::GenerationChannelMutation(
                self.library_mut(scope()?)?
                    .delete_generation_channel(&command_id, request)?,
            ),
            DesktopCommand::GenerationChannelSelect {
                command_id,
                request,
            } => DesktopData::GenerationSettingsMutation(
                self.library_mut(scope()?)?
                    .select_generation_channel_model(&command_id, request)?,
            ),
            DesktopCommand::GenerationChannelCredentialStatus {
                channel_id,
                row_version,
            } => {
                let identity = scope()?;
                let channel =
                    self.generation_channel_at_version(identity, &channel_id, row_version)?;
                let binding = crate::generation_channel_credential_binding(identity, &channel)?;
                DesktopData::GenerationCredentialStatus(GenerationCredentialStatus {
                    configured: crate::inference_credentials::exists(&binding)
                        .map_err(crate::inference_remote::credential_error)?,
                })
            }
            DesktopCommand::GenerationChannelCredentialSet {
                channel_id,
                row_version,
                api_key,
            } => {
                let identity = scope()?;
                let channel =
                    self.generation_channel_at_version(identity, &channel_id, row_version)?;
                let binding = crate::generation_channel_credential_binding(identity, &channel)?;
                let secret = crate::inference_credentials::Secret::new(api_key.0)
                    .map_err(crate::inference_remote::credential_error)?;
                crate::inference_credentials::store(&binding, &secret)
                    .map_err(crate::inference_remote::credential_error)?;
                DesktopData::Done
            }
            DesktopCommand::GenerationChannelCredentialDelete {
                channel_id,
                row_version,
            } => {
                let identity = scope()?;
                let channel =
                    self.generation_channel_at_version(identity, &channel_id, row_version)?;
                let binding = crate::generation_channel_credential_binding(identity, &channel)?;
                crate::inference_credentials::delete(&binding)
                    .map_err(crate::inference_remote::credential_error)?;
                DesktopData::Done
            }
            DesktopCommand::GenerationCredentialStatus { profile } => {
                let identity = scope()?;
                self.library(identity)?;
                let binding = legacy_generation_credential_binding(identity, &profile)?;
                DesktopData::GenerationCredentialStatus(GenerationCredentialStatus {
                    configured: crate::inference_credentials::exists(&binding)
                        .map_err(crate::inference_remote::credential_error)?,
                })
            }
            DesktopCommand::GenerationCredentialSet { profile, api_key } => {
                let identity = scope()?;
                self.library(identity)?;
                let binding = legacy_generation_credential_binding(identity, &profile)?;
                let secret = crate::inference_credentials::Secret::new(api_key.0)
                    .map_err(crate::inference_remote::credential_error)?;
                crate::inference_credentials::store(&binding, &secret)
                    .map_err(crate::inference_remote::credential_error)?;
                DesktopData::Done
            }
            DesktopCommand::GenerationCredentialDelete { profile } => {
                let identity = scope()?;
                self.library(identity)?;
                let binding = legacy_generation_credential_binding(identity, &profile)?;
                crate::inference_credentials::delete(&binding)
                    .map_err(crate::inference_remote::credential_error)?;
                DesktopData::Done
            }
            DesktopCommand::GenerationModelVerify { .. }
            | DesktopCommand::GenerationModelsList { .. }
            | DesktopCommand::GenerationChannelModelsList { .. } => {
                self.library(scope()?)?;
                return Err(AppError::InvalidInput(
                    "model verification requires native inference adapter",
                ));
            }
            DesktopCommand::GenerationSettingsSet {
                command_id,
                request,
            } => {
                // A remote single-model profile sends prompts and note text to its
                // provider; local Ollama stays on this device.
                if request
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile.provider != "ollama")
                {
                    require_remote_consent(self.library(scope()?)?)?;
                }
                DesktopData::GenerationSettingsMutation(
                    self.library_mut(scope()?)?
                        .set_generation_profile(&command_id, request)?,
                )
            }
            DesktopCommand::RetrievalSettingsSet {
                command_id,
                request,
            } => DesktopData::RetrievalSettingsMutation(
                self.library_mut(scope()?)?
                    .set_retrieval_params(&command_id, request)?,
            ),
            DesktopCommand::CleaningSettingsSet {
                command_id,
                request,
            } => DesktopData::CleaningSettingsMutation(
                self.library_mut(scope()?)?
                    .set_cleaning_params(&command_id, request)?,
            ),
            DesktopCommand::ChunkSettingsSet {
                command_id,
                request,
            } => DesktopData::ChunkSettingsMutation(
                self.library_mut(scope()?)?
                    .set_chunk_params(&command_id, request)?,
            ),
            DesktopCommand::EmbeddingProfileRevert { command_id } => {
                DesktopData::EmbeddingProfileMutation(
                    self.library_mut(scope()?)?
                        .revert_embedding_profile(&command_id)?,
                )
            }
            DesktopCommand::ChatConversationsCreate {
                command_id,
                request,
            } => DesktopData::ChatConversationMutation(
                self.library_mut(scope()?)?
                    .create_chat_conversation(&command_id, request)?,
            ),
            DesktopCommand::ChatConversationsList { after_id, limit } => {
                DesktopData::ChatConversations(
                    self.library(scope()?)?
                        .chat_conversations(after_id.as_deref(), limit)?,
                )
            }
            DesktopCommand::ChatConversationsPage { before, limit } => {
                DesktopData::ChatConversationPage(
                    self.library(scope()?)?
                        .chat_conversations_page(before, limit)?,
                )
            }
            DesktopCommand::ChatMessagesPage {
                conversation_id,
                before_ordinal,
                limit,
            } => DesktopData::ChatMessagePage(self.library(scope()?)?.chat_messages_page(
                &conversation_id,
                before_ordinal,
                limit,
            )?),
            DesktopCommand::ChatConversationsGet { conversation_id } => {
                DesktopData::ChatConversation(
                    self.library(scope()?)?
                        .chat_conversation(&conversation_id)?,
                )
            }
            DesktopCommand::ChatMessagesList {
                conversation_id,
                after_ordinal,
                limit,
            } => DesktopData::ChatMessages(self.library(scope()?)?.chat_messages(
                &conversation_id,
                after_ordinal,
                limit,
            )?),
            DesktopCommand::ChatRunsGet { run_id } => {
                DesktopData::ChatRun(self.library(scope()?)?.chat_run(&run_id)?)
            }
            DesktopCommand::ChatRunsEvents {
                run_id,
                after_seq,
                limit,
            } => DesktopData::ChatRunEvents(
                self.library(scope()?)?
                    .chat_run_events(&run_id, after_seq, limit)?,
            ),
            DesktopCommand::ChatRunsStart {
                command_id,
                request,
            } => DesktopData::ChatRunMutation(
                self.library_mut(scope()?)?
                    .accept_chat_run_from_settings(&command_id, request)?,
            ),
            DesktopCommand::ChatRunsCancel { command_id, run_id } => DesktopData::ChatRunMutation(
                self.library_mut(scope()?)?
                    .cancel_chat_run(&command_id, &run_id)?,
            ),
            DesktopCommand::KnowledgeTasksStart {
                command_id,
                request,
            } => DesktopData::KnowledgeTaskMutation(
                self.library_mut(scope()?)?
                    .start_knowledge_task(&command_id, request)?,
            ),
            DesktopCommand::KnowledgeTasksGet { task_id } => {
                DesktopData::KnowledgeTask(self.library(scope()?)?.knowledge_task(&task_id)?)
            }
            DesktopCommand::KnowledgeTasksPage(request) => {
                DesktopData::KnowledgeTasksPage(self.library(scope()?)?.knowledge_tasks(request)?)
            }
            DesktopCommand::KnowledgeTaskMaterials(request) => DesktopData::KnowledgeTaskMaterials(
                self.library(scope()?)?.knowledge_task_materials(request)?,
            ),
            DesktopCommand::KnowledgeTaskOrigins(request) => DesktopData::KnowledgeTaskOrigins(
                self.library(scope()?)?.knowledge_task_origins(request)?,
            ),
            DesktopCommand::KnowledgeTaskOutputsPage(request) => {
                DesktopData::KnowledgeTaskOutputsPage(
                    self.library(scope()?)?.knowledge_task_outputs(request)?,
                )
            }
            DesktopCommand::KnowledgeTaskTemplatesGet { template_id } => {
                DesktopData::KnowledgeTaskTemplate(
                    self.library(scope()?)?
                        .knowledge_task_template(&template_id)?,
                )
            }
            DesktopCommand::KnowledgeTaskTemplatesPage(request) => {
                DesktopData::KnowledgeTaskTemplatesPage(
                    self.library(scope()?)?.knowledge_task_templates(request)?,
                )
            }
            DesktopCommand::KnowledgeTaskTemplatesSave {
                command_id,
                request,
            } => DesktopData::KnowledgeTaskTemplateMutation(
                self.library_mut(scope()?)?
                    .save_knowledge_task_template(&command_id, request)?,
            ),
            DesktopCommand::KnowledgeTaskTemplatesDelete {
                command_id,
                request,
            } => DesktopData::KnowledgeTaskTemplateMutation(
                self.library_mut(scope()?)?
                    .delete_knowledge_task_template(&command_id, request)?,
            ),
            DesktopCommand::AnswersStart {
                command_id,
                search,
                max_output_tokens,
            } => DesktopData::AnswerMutation(
                self.library_mut(scope()?)?
                    .accept_answer_from_settings_with_budget(
                        &command_id,
                        search,
                        max_output_tokens,
                    )?,
            ),
            DesktopCommand::AnswersAgentStart {
                command_id,
                search,
                agent,
                max_output_tokens,
            } => DesktopData::AnswerMutation(
                self.library_mut(scope()?)?
                    .accept_agent_answer_from_settings_with_budget(
                        &command_id,
                        search,
                        agent,
                        max_output_tokens,
                    )?,
            ),
            DesktopCommand::AnswersGet { answer_id } => {
                DesktopData::Answer(self.library(scope()?)?.answer(&answer_id)?)
            }
            DesktopCommand::AnswersList { after_id, limit } => DesktopData::Answers(
                self.library(scope()?)?
                    .answers_page(after_id.as_deref(), limit)?,
            ),
            DesktopCommand::AnswersEvents {
                answer_id,
                after_seq,
                limit,
            } => DesktopData::AnswerEvents(
                self.library(scope()?)?
                    .answer_events(&answer_id, after_seq, limit)?,
            ),
            DesktopCommand::AnswersCancel {
                command_id,
                answer_id,
            } => DesktopData::AnswerMutation(
                self.library_mut(scope()?)?
                    .cancel_answer(&command_id, &answer_id)?,
            ),
            DesktopCommand::AnswersFeedback {
                command_id,
                request,
            } => DesktopData::AnswerMutation(
                self.library_mut(scope()?)?
                    .feedback_answer(&command_id, request)?,
            ),
            DesktopCommand::FeedbackList { limit, cursor } => DesktopData::Feedback(
                self.library(scope()?)?
                    .feedback_page(cursor.as_deref(), limit)?,
            ),
            DesktopCommand::FeedbackSummary => {
                DesktopData::FeedbackSummary(self.library(scope()?)?.feedback_summary()?)
            }
            DesktopCommand::ProposalsCreate {
                command_id,
                request,
            } => DesktopData::ProposalMutation(
                self.library_mut(scope()?)?
                    .create_proposal(&command_id, request)?,
            ),
            DesktopCommand::ProposalsGet { proposal_id } => {
                DesktopData::Proposal(self.library(scope()?)?.proposal(&proposal_id)?)
            }
            DesktopCommand::ProposalsApply {
                command_id,
                proposal_id,
            } => DesktopData::NoteCommit(
                self.library_mut(scope()?)?
                    .apply_proposal(&command_id, &proposal_id)?,
            ),
            DesktopCommand::ProposalsReject {
                command_id,
                proposal_id,
            } => DesktopData::ProposalMutation(
                self.library_mut(scope()?)?
                    .reject_proposal(&command_id, &proposal_id)?,
            ),
            DesktopCommand::SourcesList(query) => {
                DesktopData::Sources(self.library(scope()?)?.sources_page(query)?)
            }
            DesktopCommand::SourcesBrowse(query) => {
                DesktopData::SourcesBrowsePage(self.library(scope()?)?.sources_browse(query)?)
            }
            DesktopCommand::SourcesGet { source_id } => {
                DesktopData::Source(self.library(scope()?)?.source(&source_id)?)
            }
            DesktopCommand::SourcesVersion { source_version_id } => DesktopData::SourceVersion(
                self.library(scope()?)?.source_version(&source_version_id)?,
            ),
            DesktopCommand::SourcesNoteInput { source_version_id } => DesktopData::SourceNoteInput(
                self.library(scope()?)?
                    .source_note_input(&source_version_id)?,
            ),
            DesktopCommand::SourcesCreateNote {
                command_id,
                request,
            } => DesktopData::NoteCommit(
                self.library_mut(scope()?)?
                    .create_source_note(&command_id, request)?,
            ),
            DesktopCommand::SourcesVersions {
                source_id,
                after_id,
                limit,
            } => DesktopData::SourceVersions(self.library(scope()?)?.source_versions(
                &source_id,
                after_id.as_deref(),
                limit,
            )?),
            DesktopCommand::SourcesText {
                source_version_id,
                parse_id,
                offset,
                limit,
            } => DesktopData::SourceText(self.library(scope()?)?.source_text_page(
                &source_version_id,
                parse_id.as_deref(),
                offset,
                limit,
            )?),
            DesktopCommand::SourcesCover {
                source_version_id,
                parse_id,
            } => DesktopData::SourceCover(
                self.library(scope()?)?
                    .source_cover(&source_version_id, parse_id.as_deref())?,
            ),

            DesktopCommand::SourcesImage {
                source_version_id,
                parse_id,
                blob_hash,
            } => DesktopData::SourceImage(self.library(scope()?)?.source_asset(
                &source_version_id,
                &parse_id,
                &blob_hash,
            )?),
            DesktopCommand::SourcesSearch { query, limit } => {
                DesktopData::SourceSearch(self.library(scope()?)?.search_sources(&query, limit)?)
            }
            DesktopCommand::SourcesImport {
                command_id,
                request,
                tag_names,
            } => DesktopData::JobAccepted(self.import_selected_source_with_tags(
                scope()?,
                &command_id,
                request,
                tag_names,
            )?),
            DesktopCommand::SourcesImportWebpage {
                command_id,
                request,
                tag_names,
            } => DesktopData::JobAccepted(self.import_webpage_with_tags(
                scope()?,
                &command_id,
                request,
                tag_names,
            )?),
            DesktopCommand::SourceDirectoryScan { token } => DesktopData::SourceDirectoryScan(
                Box::new(self.scan_source_directory(scope()?, &token)?),
            ),
            DesktopCommand::SourcesUpdate {
                command_id,
                request,
            } => DesktopData::SourceMutation(Box::new(
                self.library_mut(scope()?)?
                    .update_source_metadata(&command_id, request)?,
            )),
            DesktopCommand::SourcesSetTrashed {
                command_id,
                request,
            } => DesktopData::SourceMutation(Box::new(
                self.library_mut(scope()?)?
                    .set_source_trashed(&command_id, request)?,
            )),
            DesktopCommand::AnnotationsGet { id } => {
                DesktopData::Annotation(self.library(scope()?)?.annotation(&id)?)
            }
            DesktopCommand::AnnotationsQuery(query) => {
                DesktopData::AnnotationPage(self.library(scope()?)?.annotations_query(query)?)
            }
            DesktopCommand::AnnotationsList {
                source_version_id,
                after_id,
                limit,
                trashed,
            } => DesktopData::Annotations(self.library(scope()?)?.annotations_page(
                &source_version_id,
                after_id.as_deref(),
                limit,
                trashed,
            )?),
            DesktopCommand::AnnotationsResolve { id } => {
                DesktopData::ResolvedAnnotation(self.library(scope()?)?.resolve_annotation(&id)?)
            }
            DesktopCommand::AnnotationsCreate {
                command_id,
                request,
            } => DesktopData::SourceMutation(Box::new(
                self.library_mut(scope()?)?
                    .create_annotation(&command_id, request)?,
            )),
            DesktopCommand::AnnotationsUpdate {
                command_id,
                request,
            } => DesktopData::SourceMutation(Box::new(
                self.library_mut(scope()?)?
                    .update_annotation(&command_id, request)?,
            )),
            DesktopCommand::AnnotationsSetTrashed {
                command_id,
                request,
            } => DesktopData::SourceMutation(Box::new(
                self.library_mut(scope()?)?
                    .set_annotation_trashed(&command_id, request)?,
            )),
            DesktopCommand::ReadingGet { source_id } => {
                DesktopData::Reading(self.library(scope()?)?.reading_state(&source_id)?)
            }
            DesktopCommand::ReadingSet {
                command_id,
                request,
            } => DesktopData::SourceMutation(Box::new(
                self.library_mut(scope()?)?
                    .set_reading_state(&command_id, request)?,
            )),
            DesktopCommand::SourceJobsList { limit } => {
                DesktopData::SourceJobs(self.library(scope()?)?.source_jobs(limit)?)
            }
            DesktopCommand::SourceJobsPage { limit, cursor } => DesktopData::SourceJobsPage(
                self.library(scope()?)?.source_jobs_page(limit, cursor)?,
            ),
            DesktopCommand::SourceJobGet { job_id } => {
                DesktopData::SourceJob(self.library(scope()?)?.source_job(&job_id)?)
            }
            DesktopCommand::SourceJobCancel { job_id } => {
                DesktopData::SourceJob(self.library_mut(scope()?)?.cancel_source_job(&job_id)?)
            }
            DesktopCommand::SourceJobRetry { command_id, job_id } => DesktopData::JobAccepted(
                self.retry_selected_source_job(scope()?, &command_id, &job_id)?,
            ),
            DesktopCommand::SourceJobEvents {
                job_id,
                after_seq,
                limit,
            } => DesktopData::SourceJobEvents(
                self.library(scope()?)?
                    .source_job_events(&job_id, after_seq, limit)?,
            ),
            DesktopCommand::NotesGet { note_id } => {
                DesktopData::Note(self.library(scope()?)?.note(&note_id)?)
            }
            DesktopCommand::BlocksSearch(request) => {
                DesktopData::Blocks(self.library(scope()?)?.blocks_page(request)?)
            }
            DesktopCommand::BlockHistory(request) => {
                DesktopData::BlockHistory(self.library(scope()?)?.block_history(request)?)
            }
            DesktopCommand::LinksResolve { input } => {
                DesktopData::ResolvedLink(self.library(scope()?)?.resolve_link(input)?)
            }
            DesktopCommand::KnowledgeGraph => {
                DesktopData::KnowledgeGraph(self.library(scope()?)?.knowledge_graph()?)
            }
            DesktopCommand::NotesBacklinks { id, limit, cursor } => DesktopData::BodyReferencePage(
                self.library(scope()?)?
                    .body_backlinks_page(&id, limit, cursor)?,
            ),
            DesktopCommand::NotesReferences {
                id,
                revision_id,
                limit,
                cursor,
            } => DesktopData::BodyReferencePage(self.library(scope()?)?.note_references_page(
                &id,
                &revision_id,
                limit,
                cursor,
            )?),
            DesktopCommand::NotesRelationBacklinks { id, limit, cursor } => {
                DesktopData::RelationBacklinkPage(
                    self.library(scope()?)?
                        .relation_backlinks_page(&id, limit, cursor)?,
                )
            }
            DesktopCommand::NotesOutgoingRelations { id } => {
                DesktopData::OutgoingRelations(self.library(scope()?)?.outgoing_relations(&id)?)
            }
            DesktopCommand::NotesCreate { command_id, note } => DesktopData::NoteCommit(
                self.library_mut(scope()?)?
                    .create_document(&command_id, note)?,
            ),
            DesktopCommand::BibliographyCatalog => {
                DesktopData::BibliographyCatalog(self.library(scope()?)?.bibliography_catalog()?)
            }
            DesktopCommand::Bibliography { note_id, style } => DesktopData::Bibliography(
                self.library(scope()?)?
                    .bibliography_styled(&note_id, style.unwrap_or_default())?,
            ),
            DesktopCommand::BibliographyImport { token } => {
                DesktopData::BibImport(self.import_bibliography_file(scope()?, &token)?)
            }
            DesktopCommand::CitationStyles => {
                DesktopData::CitationStyles(self.library(scope()?)?.citation_style_catalog()?)
            }
            DesktopCommand::CitationStyleImport { token } => {
                DesktopData::CitationStyleImport(self.import_citation_style_file(scope()?, &token)?)
            }
            DesktopCommand::CitationStyleDelete { style_id } => {
                let identity = scope()?;
                self.library_mut(identity)?
                    .delete_citation_style(&style_id)?;
                DesktopData::CitationStyles(self.library(identity)?.citation_style_catalog()?)
            }
            DesktopCommand::BibliographyWithStyle { note_id, style_id } => {
                DesktopData::Bibliography(
                    self.library(scope()?)?
                        .bibliography_with_style(&note_id, &style_id)?,
                )
            }
            DesktopCommand::NotesSave { command_id, note } => DesktopData::NoteCommit(
                self.library_mut(scope()?)?
                    .save_document(&command_id, note)?,
            ),
            DesktopCommand::NotesCopy { command_id, note } => DesktopData::NoteCommit(
                self.library_mut(scope()?)?
                    .copy_document(&command_id, note)?,
            ),
            DesktopCommand::NotesRestore { command_id, note } => DesktopData::NoteCommit(
                self.library_mut(scope()?)?
                    .restore_document(&command_id, note)?,
            ),
            DesktopCommand::RevisionsList {
                note_id,
                before,
                limit,
            } => DesktopData::Revisions(
                self.library(scope()?)?
                    .revision_page(&note_id, before, limit)?,
            ),
            DesktopCommand::RevisionGet {
                note_id,
                revision_id,
            } => DesktopData::Note(self.library(scope()?)?.revision(&note_id, &revision_id)?),
            DesktopCommand::AttachmentImport { command_id, token } => DesktopData::Attachment(
                self.import_selected_attachment(scope()?, &command_id, &token)?,
            ),
            DesktopCommand::AttachmentResume { command_id } => DesktopData::Attachment(
                self.library_mut(scope()?)?
                    .resume_attachment_import(&command_id)?,
            ),
        })
    }
}

impl AppOperationControl {
    pub fn dispatch(
        &mut self,
        request: DesktopRequest,
        active_identity: Option<&WorkspaceIdentity>,
    ) -> std::result::Result<DesktopReply, ApiError> {
        request.validate()?;
        let job_scope = || self.job_library(request.identity.as_ref(), active_identity);
        let result = match request.command {
            DesktopCommand::SourceJobsList { limit } => job_scope()
                .and_then(|library| Ok(DesktopData::SourceJobs(library.source_jobs(limit)?))),
            DesktopCommand::SourceJobsPage { limit, cursor } => job_scope().and_then(|library| {
                Ok(DesktopData::SourceJobsPage(
                    library.source_jobs_page(limit, cursor)?,
                ))
            }),
            DesktopCommand::SourceJobGet { job_id } => job_scope()
                .and_then(|library| Ok(DesktopData::SourceJob(library.source_job(&job_id)?))),
            DesktopCommand::SourceJobCancel { job_id } => job_scope().and_then(|mut library| {
                Ok(DesktopData::SourceJob(library.cancel_source_job(&job_id)?))
            }),
            DesktopCommand::SourceJobEvents {
                job_id,
                after_seq,
                limit,
            } => job_scope().and_then(|library| {
                Ok(DesktopData::SourceJobEvents(
                    library.source_job_events(&job_id, after_seq, limit)?,
                ))
            }),
            DesktopCommand::JobsList { limit } => {
                job_scope().and_then(|library| Ok(DesktopData::Jobs(library.portable_jobs(limit)?)))
            }
            DesktopCommand::JobsPage { limit, cursor } => job_scope().and_then(|library| {
                Ok(DesktopData::JobsPage(
                    library.portable_jobs_page(limit, cursor)?,
                ))
            }),
            DesktopCommand::JobGet { job_id } => {
                job_scope().and_then(|library| Ok(DesktopData::Job(library.portable_job(&job_id)?)))
            }
            DesktopCommand::JobCancel { job_id } => job_scope()
                .and_then(|library| Ok(DesktopData::Job(library.cancel_portable_job(&job_id)?))),
            DesktopCommand::OperationsList { limit } => self
                .operations(limit, active_identity)
                .map(DesktopData::Operations),
            DesktopCommand::OperationsPage { limit, cursor } => self
                .operations_page(limit, cursor, active_identity)
                .map(DesktopData::OperationsPage),
            DesktopCommand::OperationGet { operation_id } => self
                .operation(&operation_id, active_identity)
                .map(DesktopData::Operation),
            DesktopCommand::OperationCancel { operation_id } => self
                .cancel_operation(&operation_id, active_identity)
                .map(DesktopData::Operation),
            _ => Err(AppError::InvalidInput(
                "command is not an application operation query",
            )),
        }
        .map_err(|error| ApiError::from_app(error, &request.request_id))?;
        Ok(DesktopReply {
            request_id: request.request_id,
            data: result,
        })
    }
}

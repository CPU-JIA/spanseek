use crate::{
    ChatRun, ChatRunOptions, ChatRunStatus, GenerationProfile, MetadataReceipt, ProposalStatus,
    SearchEvidence,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub row_version: u32,
    /// Examples are editable/removable and never select a different execution engine.
    pub is_example: bool,
    pub created_at: String,
    pub updated_at: String,
    pub used_by_tasks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SaveKnowledgeTaskTemplate {
    pub id: Option<String>,
    pub base_row_version: Option<u32>,
    pub name: String,
    pub description: String,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DeleteKnowledgeTaskTemplate {
    pub id: String,
    pub base_row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskTemplateMutation {
    pub template: KnowledgeTaskTemplate,
    pub deleted: bool,
    pub receipt: MetadataReceipt,
}
impl crate::mutations::RecordedResult for KnowledgeTaskTemplateMutation {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskTemplatesRequest {
    pub page: u32,
    pub limit: u32,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskTemplatesPage {
    pub items: Vec<KnowledgeTaskTemplate>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskTemplateRef {
    pub id: String,
    pub row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskTemplateSnapshot {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub row_version: u32,
    pub is_example: bool,
}
impl From<&KnowledgeTaskTemplate> for KnowledgeTaskTemplateSnapshot {
    fn from(template: &KnowledgeTaskTemplate) -> Self {
        Self {
            id: template.id.clone(),
            name: template.name.clone(),
            description: template.description.clone(),
            instructions: template.instructions.clone(),
            row_version: template.row_version,
            is_example: template.is_example,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskPeriod {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StartKnowledgeTask {
    pub title: String,
    pub instructions: String,
    /// A template helps compose the request; a task never requires one.
    pub template: Option<KnowledgeTaskTemplateRef>,
    /// A reporting period, not a claim about when every input was created.
    pub period: Option<KnowledgeTaskPeriod>,
    pub materials: Vec<SearchEvidence>,
    pub generation_settings_version: u32,
    pub options: Option<ChatRunOptions>,
    pub language: String,
    pub retry_of: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskOutput {
    pub id: String,
    pub proposal_id: Option<String>,
    pub title: String,
    pub status: ProposalStatus,
    pub target_note_id: Option<String>,
    pub applied_note_id: Option<String>,
    pub applied_revision_id: Option<String>,
    pub note_available: bool,
    pub note_trashed: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTask {
    pub id: String,
    pub input: StartKnowledgeTask,
    pub template_snapshot: Option<KnowledgeTaskTemplateSnapshot>,
    pub template_current: bool,
    pub channel_name: Option<String>,
    pub run: ChatRun,
    pub output_text: String,
    pub latest_output: Option<KnowledgeTaskOutput>,
    pub output_count: u32,
    pub materials_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskSummary {
    pub id: String,
    pub title: String,
    pub template_name: Option<String>,
    pub status: ChatRunStatus,
    pub model: Option<GenerationProfile>,
    pub channel_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub output_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTasksRequest {
    pub template_id: Option<String>,
    pub page: u32,
    pub limit: u32,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTasksPage {
    pub items: Vec<KnowledgeTaskSummary>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskOutputsRequest {
    pub task_id: String,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskOutputsPage {
    pub items: Vec<KnowledgeTaskOutput>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskMutation {
    pub task: KnowledgeTask,
    pub receipt: MetadataReceipt,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskMaterialTarget {
    pub object_id: String,
    pub content_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskMaterialsRequest {
    pub targets: Vec<KnowledgeTaskMaterialTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTaskMaterialStatus {
    Complete,
    Limited,
    Empty,
    Missing,
    Trashed,
    SearchDisabled,
    Unparsed,
    Changed,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskMaterialCoverage {
    pub object_id: String,
    pub title: String,
    pub status: KnowledgeTaskMaterialStatus,
    pub total_characters: Option<u32>,
    pub included_characters: u32,
    pub included_fragments: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskMaterials {
    pub materials: Vec<SearchEvidence>,
    pub coverage: Vec<KnowledgeTaskMaterialCoverage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeTaskOriginsRequest {
    pub note_id: String,
    pub page: u32,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskOrigin {
    pub task_id: String,
    pub task_title: String,
    pub applied_revision_id: String,
    pub task_created_at: String,
    pub materials_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct KnowledgeTaskOriginsPage {
    pub items: Vec<KnowledgeTaskOrigin>,
    pub total: u32,
    pub page: u32,
    pub limit: u32,
}
impl crate::mutations::RecordedResult for KnowledgeTaskMutation {
    fn receipt_mut(&mut self) -> &mut MetadataReceipt {
        &mut self.receipt
    }
}

use crate::{EvidenceKind, SearchEvidence};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportTemplate {
    pub schema_version: u32,
    pub name: String,
    pub title: String,
    pub question: String,
    pub section_heading: String,
    pub paragraph: String,
    pub sources_heading: String,
    pub source_line: String,
    pub citation_format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportCitation {
    pub id: String,
    pub evidence: SearchEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSource {
    pub citation_id: String,
    pub object_id: String,
    pub kind: EvidenceKind,
    pub title: String,
    pub content_revision: String,
    pub locator: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportDraft {
    pub schema_version: u32,
    pub title: String,
    pub question: String,
    pub generated_by: String,
    pub template_name: String,
    pub markdown: String,
    pub citations: Vec<ReportCitation>,
    pub sources: Vec<ReportSource>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportVerification {
    pub total: u32,
    pub valid: u32,
    pub invalid_citation_ids: Vec<String>,
    pub all_valid: bool,
}

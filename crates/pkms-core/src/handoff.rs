//! Portable resume handoff used by desktop CLI/UI and the mobile PWA bridge.

use crate::{CreateNote, DOCUMENT_SCHEMA, Error, Library, Note, NoteDocument, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub const RESUME_HANDOFF_FORMAT: &str = "spanseek-resume-handoff-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResumeHandoff {
    pub format: String,
    pub workspace_id: String,
    #[serde(default)]
    pub exported_at: String,
    #[serde(default)]
    pub objects: Vec<HandoffObject>,
    /// Preserved by clients that also carry excerpts; desktop import ignores it.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub excerpts: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffObject {
    pub object_id: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub base_version: u32,
    #[serde(default)]
    pub base_revision_id: Option<String>,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub reading_position: HandoffReadingPosition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_schema: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<NoteDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffReadingPosition {
    #[serde(default = "scroll_kind")]
    pub kind: String,
    #[serde(default)]
    pub position: u64,
}

impl Default for HandoffReadingPosition {
    fn default() -> Self {
        Self {
            kind: scroll_kind(),
            position: 0,
        }
    }
}

fn scroll_kind() -> String {
    "scroll".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffImport {
    pub imported: Vec<Note>,
    pub skipped_existing: Vec<String>,
}

impl Library {
    pub fn export_resume_handoff(
        &self,
        object_ids: &[String],
        positions: &HashMap<String, u64>,
    ) -> Result<ResumeHandoff> {
        let info = self.info()?;
        let notes = if object_ids.is_empty() {
            self.list_notes()?
        } else {
            object_ids
                .iter()
                .map(|id| self.note(id))
                .collect::<Result<Vec<_>>>()?
        };
        let mut objects = Vec::new();
        for note in notes {
            if note.trashed {
                continue;
            }
            objects.push(HandoffObject {
                object_id: note.id.clone(),
                title: note.title.clone(),
                body: note.document.plain_text(),
                source_url: String::new(),
                base_version: note.revision,
                base_revision_id: Some(note.revision_id.clone()),
                updated_at: note.updated_at.clone(),
                reading_position: HandoffReadingPosition {
                    kind: scroll_kind(),
                    position: positions.get(&note.id).copied().unwrap_or(0),
                },
                document_schema: Some(note.schema_version),
                document: Some(note.document),
            });
        }
        Ok(ResumeHandoff {
            format: RESUME_HANDOFF_FORMAT.into(),
            workspace_id: info.workspace_id,
            exported_at: chrono::Utc::now().to_rfc3339(),
            objects,
            excerpts: serde_json::Value::Null,
        })
    }

    /// Imports a handoff without overwriting an existing object. Existing IDs
    /// are skipped only when their content matches; a mismatched ID fails closed.
    pub fn import_resume_handoff(&mut self, handoff: &ResumeHandoff) -> Result<HandoffImport> {
        if handoff.format != RESUME_HANDOFF_FORMAT {
            return Err(Error::InvalidInput("unsupported resume handoff format"));
        }
        let info = self.info()?;
        if handoff.workspace_id != info.workspace_id {
            return Err(Error::InvalidInput(
                "resume handoff belongs to another workspace",
            ));
        }
        // Validate the whole bundle before creating any object. A malformed or
        // conflicting later object must not leave an earlier partial import.
        for object in &handoff.objects {
            crate::document::valid_uuid(&object.object_id)?;
            if self.contains_object(&object.object_id)? {
                let existing = self.note(&object.object_id)?;
                let same_document = object
                    .document
                    .as_ref()
                    .is_some_and(|document| document == &existing.document);
                let same_text =
                    existing.title == object.title && existing.document.plain_text() == object.body;
                if !same_document && !same_text {
                    return Err(Error::InvalidInput(
                        "resume handoff object conflicts with an existing object",
                    ));
                }
            }
        }
        let mut imported = Vec::new();
        let mut skipped_existing = Vec::new();
        for object in &handoff.objects {
            if self.contains_object(&object.object_id)? {
                skipped_existing.push(object.object_id.clone());
                continue;
            }
            let schema = object.document_schema.unwrap_or(DOCUMENT_SCHEMA);
            let document = object
                .document
                .clone()
                .unwrap_or_else(|| NoteDocument::from_text(object.object_id.clone(), &object.body));
            let commit = self.create_document_with_id(
                &Uuid::new_v4().to_string(),
                &object.object_id,
                CreateNote {
                    collection_id: None,
                    base_collection_row_version: None,
                    title: object.title.clone(),
                    document_schema: schema,
                    document,
                },
            )?;
            imported.push(commit.note);
        }
        Ok(HandoffImport {
            imported,
            skipped_existing,
        })
    }
}

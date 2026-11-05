//! Explicit, recoverable creation of an editable note from an immutable text original.
use crate::{
    CreateSourceNote, DOCUMENT_SCHEMA, DocumentNode, Error, Library, NodeAttributes, NodeType,
    NoteCommit, Result, SourceFormat, SourceNoteInput, notes,
};
use rusqlite::TransactionBehavior;
use sha2::{Digest, Sha256};
use std::io::Read;

const MAX_EDITABLE_SOURCE_BYTES: u64 = 1024 * 1024;

impl Library {
    pub fn source_note_input(&self, version_id: &str) -> Result<SourceNoteInput> {
        let (version, original) = self.source_original_file(version_id)?;
        if !matches!(version.format, SourceFormat::Text | SourceFormat::Markdown) {
            return Err(Error::InvalidInput(
                "only Markdown and TXT can create editable notes",
            ));
        }
        if version.size > MAX_EDITABLE_SOURCE_BYTES {
            return Err(Error::InvalidInput(
                "editable note import supports originals up to 1 MiB",
            ));
        }
        if self.source(&version.source_id)?.trashed {
            return Err(Error::Trashed);
        }
        let mut bytes = Vec::with_capacity(version.size as usize);
        original
            .take(MAX_EDITABLE_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != version.size
            || format!("{:x}", Sha256::digest(&bytes)) != version.blob_hash
        {
            return Err(Error::IntegrityFailure);
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            Error::InvalidInput("original is not UTF-8; convert its encoding before importing")
        })?;
        Ok(SourceNoteInput { version, text })
    }

    pub fn create_source_note(
        &mut self,
        command_id: &str,
        mut request: CreateSourceNote,
    ) -> Result<NoteCommit> {
        request.title = request.title.trim().to_owned();
        let hash = notes::request_hash(command_id, "sources.create_note", &request)?;
        // A confirmed result stays replayable even if the source is later trashed or damaged.
        if let Some(result) = notes::replay(&self.connection, command_id, &hash)? {
            return Ok(result);
        }
        if !matches!(request.document_schema, 2 | DOCUMENT_SCHEMA) {
            return Err(Error::InvalidInput(
                "editable source import requires document schema 2 or 3",
            ));
        }
        request.document.validate(request.document_schema)?;
        let original = self.source_note_input(&request.source_version_id)?;
        if request.source_blob_hash != original.version.blob_hash {
            return Err(Error::InvalidInput("source original hash does not match"));
        }
        let source_id = original.version.source_id;
        let mut provenance = crate::NoteDocument::from_text(uuid::Uuid::new_v4().to_string(), "")
            .content
            .remove(0);
        provenance.content.push(DocumentNode {
            kind: NodeType::NoteReference,
            attrs: NodeAttributes {
                target_id: Some(source_id.clone()),
                target_revision_id: Some(request.source_version_id),
                occurrence_id: Some(uuid::Uuid::new_v4().to_string()),
                label: Some(original.version.original_name),
                ..Default::default()
            },
            content: vec![],
            text: None,
            marks: vec![],
        });
        request.document.content.push(provenance);
        let note_id = uuid::Uuid::new_v4().to_string();
        self.resolve_wiki_document(Some(&note_id), &request.title, &mut request.document)?;
        // Imported documents obey the same attachment integrity rules as the editor.
        self.verify_document_attachments(&request.document)?;
        let prepared = notes::Prepared::new(
            &request.title,
            request.document_schema,
            request.document,
            None,
        )?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = notes::replay(&tx, command_id, &hash)? {
            return Ok(result);
        }
        if crate::sources::read_source(&tx, &source_id)?.trashed {
            return Err(Error::Trashed);
        }
        let note = notes::commit_revision(&tx, &note_id, None, prepared, "human")?;
        let result = notes::receipt(&tx, command_id, &hash, "sources.create_note", note)?;
        tx.commit()?;
        Ok(result)
    }
}

use crate::{
    CommitReceipt, CopyNote, CreateNote, DOCUMENT_SCHEMA, Error, Library, NodeType, Note,
    NoteCommit, NoteDocument, RestoreNote, Result, SaveNote, document::valid_uuid, index_note,
    insert_note_index, tokenize, validate_text,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) fn content_hash(title: &str, schema: u32, json: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(title, schema, json)).expect("string serialization"))
    )
}

pub(crate) struct Prepared {
    title: String,
    schema: u32,
    document: NoteDocument,
    json: String,
    hash: String,
    type_key: Option<String>,
    title_tokens: String,
    body_tokens: String,
}

impl Prepared {
    pub(crate) fn new(
        title: &str,
        schema: u32,
        document: NoteDocument,
        original_json: Option<String>,
    ) -> Result<Self> {
        validate_text(title, "")?;
        document.validate(schema)?;
        let json = original_json.unwrap_or(serde_json::to_string(&document)?);
        let title = title.trim().to_owned();
        let hash = content_hash(&title, schema, &json);
        let type_key = crate::queries::frontmatter_type_key(&document);
        let title_tokens = tokenize(&title);
        let body_tokens = tokenize(&document.plain_text());
        Ok(Self {
            title,
            schema,
            document,
            json,
            hash,
            type_key,
            title_tokens,
            body_tokens,
        })
    }
}

impl Library {
    pub fn create_document(&mut self, command_id: &str, request: CreateNote) -> Result<NoteCommit> {
        let id = Uuid::new_v4().to_string();
        self.create_document_at(command_id, id, request)
    }

    /// Creates a note with a caller-supplied stable object ID. Handoff import
    /// uses this so a mobile capture keeps its identity across the bridge.
    pub fn create_document_with_id(
        &mut self,
        command_id: &str,
        id: &str,
        request: CreateNote,
    ) -> Result<NoteCommit> {
        valid_uuid(id)?;
        if self.contains_object(id)? {
            return Err(Error::InvalidInput("object ID already exists"));
        }
        self.create_document_at(command_id, id.to_owned(), request)
    }

    fn create_document_at(
        &mut self,
        command_id: &str,
        id: String,
        mut request: CreateNote,
    ) -> Result<NoteCommit> {
        request.title = request.title.trim().to_owned();
        request.document.validate(request.document_schema)?;
        let hash = request_hash(command_id, "notes.create", &request)?;
        if let Some(result) = replay(&self.connection, command_id, &hash)? {
            return Ok(result);
        }
        self.resolve_wiki_document(Some(&id), &request.title, &mut request.document)?;
        let prepared = Prepared::new(
            &request.title,
            request.document_schema,
            request.document,
            None,
        )?;
        self.verify_document_attachments(&prepared.document)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash)? {
            return Ok(result);
        }
        let collection = match (
            request.collection_id.as_deref(),
            request.base_collection_row_version,
        ) {
            (None, None) => None,
            (Some(id), Some(base)) => Some((id, base)),
            _ => {
                return Err(Error::InvalidInput(
                    "collection ID and base version must be paired",
                ));
            }
        };
        let note = commit_revision(&tx, &id, None, prepared, "human")?;
        if let Some((collection_id, base)) = collection {
            crate::collections::attach_new_note(&tx, collection_id, base, &id)?;
        }
        let result = receipt(&tx, command_id, &hash, "notes.create", note)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn save_document(&mut self, command_id: &str, mut request: SaveNote) -> Result<NoteCommit> {
        valid_uuid(&request.note_id)?;
        valid_uuid(&request.base_revision_id)?;
        request.title = request.title.trim().to_owned();
        request.document.validate(request.document_schema)?;
        let hash = request_hash(command_id, "notes.save", &request)?;
        if let Some(result) = replay(&self.connection, command_id, &hash)? {
            return Ok(result);
        }
        self.resolve_wiki_document(
            Some(&request.note_id),
            &request.title,
            &mut request.document,
        )?;
        let prepared = Prepared::new(
            &request.title,
            request.document_schema,
            request.document,
            None,
        )?;
        self.verify_document_attachments(&prepared.document)?;
        #[cfg(test)]
        fault_tests::checkpoint("before_transaction", &self.connection);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash)? {
            return Ok(result);
        }
        let current = checked_head(&tx, &request.note_id, &request.base_revision_id)?;
        let note = commit_revision(&tx, &request.note_id, Some(&current), prepared, "human")?;
        #[cfg(test)]
        fault_tests::checkpoint("before_receipt", &tx);
        let result = receipt(&tx, command_id, &hash, "notes.save", note)?;
        tx.commit()?;
        #[cfg(test)]
        fault_tests::checkpoint("after_commit", &self.connection);
        Ok(result)
    }

    pub fn restore_document(
        &mut self,
        command_id: &str,
        request: RestoreNote,
    ) -> Result<NoteCommit> {
        valid_uuid(&request.note_id)?;
        valid_uuid(&request.base_revision_id)?;
        valid_uuid(&request.target_revision_id)?;
        let hash = request_hash(command_id, "notes.restore_revision", &request)?;
        if let Some(result) = replay(&self.connection, command_id, &hash)? {
            return Ok(result);
        }
        // Prepare immutable historical input outside the write transaction.
        let target = revision(
            &self.connection,
            &request.note_id,
            &request.target_revision_id,
        )?;
        let json = self.connection.query_row(
            "SELECT document_json FROM note_revisions WHERE revision_id=?1",
            [&request.target_revision_id],
            |r| r.get(0),
        )?;
        let prepared = Prepared::new(
            &target.title,
            target.schema_version,
            target.document,
            Some(json),
        )?;
        self.verify_document_attachments(&prepared.document)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash)? {
            return Ok(result);
        }
        let current = checked_head(&tx, &request.note_id, &request.base_revision_id)?;
        let note = commit_revision(&tx, &request.note_id, Some(&current), prepared, "restore")?;
        let result = receipt(&tx, command_id, &hash, "notes.restore_revision", note)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn copy_document(&mut self, command_id: &str, mut request: CopyNote) -> Result<NoteCommit> {
        valid_uuid(&request.note_id)?;
        valid_uuid(&request.source_revision_id)?;
        request.title = request.title.trim().to_owned();
        let hash = request_hash(command_id, "notes.copy", &request)?;
        // Fast replay avoids reconstructing potentially large copied documents.
        if let Some(result) = replay(&self.connection, command_id, &hash)? {
            return Ok(result);
        }
        let source = revision(
            &self.connection,
            &request.note_id,
            &request.source_revision_id,
        )?;
        let id = Uuid::new_v4().to_string();
        let document = source.document.copy_with_new_ids(&request.note_id, &id);
        let prepared = Prepared::new(&request.title, DOCUMENT_SCHEMA, document, None)?;
        self.verify_document_attachments(&prepared.document)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = replay(&tx, command_id, &hash)? {
            return Ok(result);
        }
        if read_note(&tx, &request.note_id)?.trashed {
            return Err(Error::Trashed);
        }
        let note = commit_revision(&tx, &id, None, prepared, "copy")?;
        let result = receipt(&tx, command_id, &hash, "notes.copy", note)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn revision(&self, note_id: &str, revision_id: &str) -> Result<Note> {
        revision(&self.connection, note_id, revision_id)
    }
}

pub(crate) fn request_hash(
    command_id: &str,
    kind: &str,
    request: &impl Serialize,
) -> Result<String> {
    valid_uuid(command_id)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(1, kind, request))?)
    ))
}

pub(crate) fn checked_head(connection: &Connection, id: &str, base: &str) -> Result<Note> {
    let note = read_note(connection, id)?;
    if note.trashed {
        return Err(Error::Trashed);
    }
    if note.revision_id != base {
        return Err(Error::RevisionConflict {
            expected: base.into(),
            actual: note.revision_id,
        });
    }
    Ok(note)
}

pub(crate) fn replay(
    connection: &Connection,
    command_id: &str,
    hash: &str,
) -> Result<Option<NoteCommit>> {
    let row = connection.query_row("SELECT payload_hash,object_id,revision_id,change_seq,committed_at,command_kind FROM command_receipts WHERE command_id=?1", [command_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).optional()?;
    let Some((expected, object_id, revision_id, seq, committed_at, kind)) = row else {
        return Ok(None);
    };
    if expected != hash {
        return Err(Error::IdempotencyMismatch);
    }
    if kind == crate::trash::PURGED_RECEIPT_KIND {
        return Err(Error::NotFound);
    }
    let revision_id = revision_id.ok_or(Error::IntegrityFailure)?;
    let mut note = revision(connection, &object_id, &revision_id)?;
    // Receipt describes the original immutable commit, not the current lifecycle.
    note.trashed = false;
    Ok(Some(NoteCommit {
        note,
        receipt: CommitReceipt {
            command_id: command_id.into(),
            object_id,
            revision_id,
            change_seq: seq.to_string(),
            committed_at,
            replayed: true,
        },
    }))
}

pub(crate) fn receipt(
    connection: &Connection,
    command_id: &str,
    hash: &str,
    kind: &str,
    note: Note,
) -> Result<NoteCommit> {
    let seq: i64 = connection.query_row(
        "SELECT next_change_seq FROM workspace_meta WHERE singleton=1",
        [],
        |r| r.get(0),
    )?;
    let next = seq
        .checked_add(1)
        .ok_or(Error::InvalidInput("change sequence overflow"))?;
    connection.execute(
        "UPDATE workspace_meta SET next_change_seq=?1 WHERE singleton=1",
        [next],
    )?;
    connection.execute("INSERT INTO command_receipts(command_id,payload_hash,command_kind,object_id,revision_id,change_seq,committed_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![command_id,hash,kind,note.id,note.revision_id,seq,note.updated_at])?;
    let receipt = CommitReceipt {
        command_id: command_id.into(),
        object_id: note.id.clone(),
        revision_id: note.revision_id.clone(),
        change_seq: seq.to_string(),
        committed_at: note.updated_at.clone(),
        replayed: false,
    };
    Ok(NoteCommit { note, receipt })
}

pub(crate) fn commit_revision(
    connection: &Connection,
    id: &str,
    current: Option<&Note>,
    prepared: Prepared,
    author: &str,
) -> Result<Note> {
    let ordinal = current
        .map_or(Some(1), |n| n.revision.checked_add(1))
        .ok_or(Error::InvalidInput("revision overflow"))?;
    let revision_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    for node in prepared
        .document
        .nodes()
        .into_iter()
        .filter(|node| !node.attrs.id.is_empty())
    {
        let owner: Option<String> = connection
            .query_row(
                "SELECT note_id FROM blocks WHERE id=?1",
                [&node.attrs.id],
                |r| r.get(0),
            )
            .optional()?;
        let object_collision: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE id=?1)",
            [&node.attrs.id],
            |r| r.get(0),
        )?;
        if owner.as_ref().is_some_and(|owner| owner != id) || object_collision {
            return Err(Error::InvalidDocument(
                "block identity belongs to another object",
            ));
        }
    }
    if current.is_none() {
        connection.execute("INSERT INTO notes(id,title,current_revision,anchor_id,updated_at,type_key) VALUES(?1,?2,1,?3,?4,?5)",params![id,prepared.title,prepared.document.content[0].attrs.id,now,prepared.type_key])?;
        connection.execute(
            "INSERT INTO objects(id,kind,inbox_state) VALUES(?1,'note','pending')",
            [id],
        )?;
    }
    connection.execute("INSERT INTO note_revisions(revision_id,note_id,revision,parent_revision_id,title,schema_version,document_json,content_hash,author_kind,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![revision_id,id,ordinal,current.map(|n| &n.revision_id),prepared.title,prepared.schema,prepared.json,prepared.hash,author,now])?;
    if current.is_some() {
        connection.execute("DELETE FROM body_links WHERE source_id=?1", [id])?;
    }
    crate::blocks::replace_block_metadata(connection, id, &revision_id, &prepared.document, &now)?;
    for node in prepared.document.nodes() {
        if !node.attrs.id.is_empty() {
            connection.execute(
                "INSERT INTO note_revision_blocks(revision_id,block_id) VALUES(?1,?2)",
                params![revision_id, node.attrs.id],
            )?;
        }
        if node.is_body_reference() {
            connection.execute("INSERT INTO body_links(source_id,source_revision_id,occurrence_id,target_id,target_block_id,target_revision_id,context) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,revision_id,node.attrs.occurrence_id,node.attrs.target_id,node.attrs.target_block_id,node.attrs.target_revision_id,node.plain_text()])?;
        }
        if node.kind == NodeType::Image {
            connection.execute("INSERT INTO note_revision_blobs(revision_id,blob_hash,node_id,original_name) VALUES(?1,?2,?3,?4)",params![revision_id,node.attrs.blob_hash,node.attrs.id,node.attrs.original_name])?;
        }
    }
    connection.execute("UPDATE notes SET title=?2,current_revision=?3,current_revision_id=?4,updated_at=?5,type_key=?6 WHERE id=?1",params![id,prepared.title,ordinal,revision_id,now,prepared.type_key])?;
    if current.is_some() {
        connection.execute(
            "UPDATE objects SET row_version=row_version+1 WHERE id=?1",
            [id],
        )?;
    }
    let write_index = if current.is_some() {
        index_note
    } else {
        insert_note_index
    };
    write_index(
        connection,
        id,
        &prepared.title_tokens,
        &prepared.body_tokens,
    )?;
    connection.execute("INSERT INTO note_index_queue(note_id,revision_id,queued_at) VALUES(?1,?2,?3) ON CONFLICT(note_id) DO UPDATE SET revision_id=excluded.revision_id,queued_at=excluded.queued_at",params![id,revision_id,now])?;
    revision(connection, id, &revision_id)
}

pub(crate) fn read_note(connection: &Connection, id: &str) -> Result<Note> {
    let head: String = connection
        .query_row(
            "SELECT current_revision_id FROM notes WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    revision(connection, id, &head)
}

pub(crate) fn revision(connection: &Connection, id: &str, revision_id: &str) -> Result<Note> {
    let row = connection.query_row("SELECT r.title,r.revision,r.document_json,n.trashed,r.created_at,r.schema_version,r.parent_revision_id,r.content_hash,r.author_kind,(SELECT MIN(created_at) FROM note_revisions WHERE note_id=n.id) FROM note_revisions r JOIN notes n ON n.id=r.note_id WHERE r.note_id=?1 AND r.revision_id=?2",params![id,revision_id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?,r.get::<_,u32>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?,r.get::<_,String>(8)?,r.get::<_,String>(9)?))).optional()?.ok_or(Error::NotFound)?;
    if content_hash(&row.0, row.5, &row.2) != row.7 {
        return Err(Error::IntegrityFailure);
    }
    let document: NoteDocument = serde_json::from_str(&row.2)?;
    document.validate(row.5)?;
    Ok(Note {
        id: id.into(),
        title: row.0,
        revision: row.1,
        document,
        trashed: row.3,
        created_at: row.9,
        updated_at: row.4,
        schema_version: row.5,
        revision_id: revision_id.into(),
        parent_revision_id: row.6,
        content_hash: row.7,
        author_kind: row.8,
    })
}

#[cfg(test)]
#[path = "notes_fault_tests.rs"]
mod fault_tests;

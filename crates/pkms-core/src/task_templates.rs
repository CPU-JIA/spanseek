use crate::{
    DeleteKnowledgeTaskTemplate, Error, KnowledgeTaskTemplate, KnowledgeTaskTemplateMutation,
    KnowledgeTaskTemplatesPage, KnowledgeTaskTemplatesRequest, Library, Result,
    SaveKnowledgeTaskTemplate, document::valid_uuid, mutations, organization::normalize_name,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

pub(crate) fn fields(
    name: &str,
    description: &str,
    instructions: &str,
) -> Result<(String, String, String, String)> {
    let (name, normalized) =
        normalize_name(name).map_err(|_| Error::InvalidInput("task_template_name_invalid"))?;
    let description = description.trim();
    let instructions = instructions.trim();
    if description.chars().count() > 2000
        || description.contains('\0')
        || instructions.is_empty()
        || instructions.chars().count() > 8000
        || instructions.contains('\0')
    {
        return Err(Error::InvalidInput("task_template_content_invalid"));
    }
    Ok((name, normalized, description.into(), instructions.into()))
}

pub(crate) fn read(db: &Connection, id: &str) -> Result<KnowledgeTaskTemplate> {
    valid_uuid(id)?;
    db.query_row("SELECT id,name,description,instructions,row_version,is_example,created_at,updated_at,
        (SELECT count(*) FROM knowledge_tasks t WHERE json_extract(t.template_snapshot_json,'$.id')=p.id)
        FROM knowledge_task_templates p WHERE id=?1",[id],|r|Ok(KnowledgeTaskTemplate { id:r.get(0)?,name:r.get(1)?,description:r.get(2)?,instructions:r.get(3)?,row_version:r.get(4)?,is_example:r.get(5)?,created_at:r.get(6)?,updated_at:r.get(7)?,used_by_tasks:r.get(8)? }))
        .optional()?.ok_or(Error::NotFound)
}

impl Library {
    pub fn knowledge_task_template(&self, id: &str) -> Result<KnowledgeTaskTemplate> {
        read(&self.connection, id)
    }

    pub fn knowledge_task_templates(
        &self,
        request: KnowledgeTaskTemplatesRequest,
    ) -> Result<KnowledgeTaskTemplatesPage> {
        crate::collections::validate_limit(request.limit)?;
        if request.page == 0 {
            return Err(Error::InvalidInput("knowledge_task_page_invalid"));
        }
        let query = request.query.unwrap_or_default().trim().to_lowercase();
        if query.chars().count() > 512 || query.chars().any(char::is_control) {
            return Err(Error::InvalidInput("knowledge_task_query_invalid"));
        }
        let snapshot = self.connection.unchecked_transaction()?;
        let total:u32=snapshot.query_row("SELECT count(*) FROM knowledge_task_templates WHERE instr(normalized_name,?1)>0 OR instr(lower(description),?1)>0",[&query],|r|r.get(0))?;
        let page = request.page.min(total.max(1).div_ceil(request.limit));
        let ids=snapshot.prepare("SELECT id FROM knowledge_task_templates WHERE instr(normalized_name,?1)>0 OR instr(lower(description),?1)>0 ORDER BY normalized_name,id LIMIT ?2 OFFSET ?3")?
            .query_map(params![query,request.limit,i64::from(page-1)*i64::from(request.limit)],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let items = ids
            .iter()
            .map(|id| read(&snapshot, id))
            .collect::<Result<Vec<_>>>()?;
        Ok(KnowledgeTaskTemplatesPage {
            items,
            total,
            page,
            limit: request.limit,
        })
    }

    pub fn save_knowledge_task_template(
        &mut self,
        command: &str,
        request: SaveKnowledgeTaskTemplate,
    ) -> Result<KnowledgeTaskTemplateMutation> {
        let kind = "knowledge_task_templates.save";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(result);
        }
        let (name, normalized, description, instructions) =
            fields(&request.name, &request.description, &request.instructions)?;
        let id = match (&request.id, request.base_row_version) {
            (None, None) => Uuid::new_v4().to_string(),
            (Some(id), Some(version)) => {
                let old = read(&tx, id)?;
                mutations::check_version(id, version, old.row_version)?;
                id.clone()
            }
            _ => return Err(Error::InvalidInput("task_template_version_invalid")),
        };
        let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM knowledge_task_templates WHERE normalized_name=?1 AND id<>?2)",params![normalized,id],|r|r.get(0))?;
        if duplicate {
            return Err(Error::InvalidInput("task_template_duplicate_name"));
        }
        let now = Utc::now().to_rfc3339();
        if request.id.is_some() {
            let old = read(&tx, &id)?;
            if old.name != name
                || old.description != description
                || old.instructions != instructions
            {
                let version = old
                    .row_version
                    .checked_add(1)
                    .ok_or(Error::IntegrityFailure)?;
                tx.execute("UPDATE knowledge_task_templates SET name=?2,normalized_name=?3,description=?4,instructions=?5,row_version=?6,updated_at=?7 WHERE id=?1",params![id,name,normalized,description,instructions,version,now])?;
            }
        } else {
            tx.execute(
                "INSERT INTO knowledge_task_templates VALUES(?1,?2,?3,?4,?5,1,0,?6,?6)",
                params![id, name, normalized, description, instructions, now],
            )?;
        }
        let result = KnowledgeTaskTemplateMutation {
            template: read(&tx, &id)?,
            deleted: false,
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(&tx, &hash, kind, &id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn delete_knowledge_task_template(
        &mut self,
        command: &str,
        request: DeleteKnowledgeTaskTemplate,
    ) -> Result<KnowledgeTaskTemplateMutation> {
        let kind = "knowledge_task_templates.delete";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(result) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(result);
        }
        let template = read(&tx, &request.id)?;
        mutations::check_version(&request.id, request.base_row_version, template.row_version)?;
        tx.execute(
            "DELETE FROM knowledge_task_templates WHERE id=?1",
            [&request.id],
        )?;
        let result = KnowledgeTaskTemplateMutation {
            template,
            deleted: true,
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(&tx, &hash, kind, &request.id, &result.receipt, &result)?;
        tx.commit()?;
        Ok(result)
    }
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let ids = db
        .prepare("SELECT id,normalized_name FROM knowledge_task_templates")?
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, normalized) in ids {
        let template = read(db, &id)?;
        let (name, expected, description, instructions) = fields(
            &template.name,
            &template.description,
            &template.instructions,
        )?;
        if template.name != name
            || normalized != expected
            || template.description != description
            || template.instructions != instructions
            || template.row_version == 0
            || chrono::DateTime::parse_from_rfc3339(&template.created_at).is_err()
            || chrono::DateTime::parse_from_rfc3339(&template.updated_at).is_err()
        {
            return Err(Error::IntegrityFailure);
        }
    }
    Ok(())
}

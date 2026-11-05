//! Read-only global knowledge graph projection over live body links, linked
//! blocks and canvas membership. Historical revisions and trashed objects never
//! leak into the view. Every query is bounded before layout work begins.

use crate::{Library, Result, Tag};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

const MAX_GRAPH_NODES: usize = 10_000;
const MAX_GRAPH_EDGES: usize = 50_000;
const MAX_EDGE_CONTEXT_CHARS: usize = 180;
const MAX_BLOCK_TITLE_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeGraphNodeKind {
    Note,
    Source,
    Annotation,
    Block,
    Canvas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeGraphEdgeKind {
    Reference,
    Relation,
    BlockReference,
    Canvas,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KnowledgeGraphNode {
    /// Graph-local identity. Object nodes use their UUID; blocks and canvases
    /// use a typed prefix so two tables cannot collide.
    pub id: String,
    pub title: String,
    pub kind: KnowledgeGraphNodeKind,
    pub tag_ids: Vec<String>,
    /// Collections this object belongs to. Block nodes inherit their note's
    /// memberships; canvas nodes are not collection members.
    pub collection_ids: Vec<String>,
    pub degree: u32,
    /// Object to open. For a block this is its note; for a canvas it is the canvas.
    pub object_id: Option<String>,
    /// Exact block identity when `kind` is `block`.
    pub block_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KnowledgeGraphEdge {
    pub source_id: String,
    pub target_id: String,
    pub kind: KnowledgeGraphEdgeKind,
    pub weight: u32,
    pub context: String,
    pub target_block_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct KnowledgeGraph {
    pub nodes: Vec<KnowledgeGraphNode>,
    pub edges: Vec<KnowledgeGraphEdge>,
    /// Tag vocabulary used by this bounded projection, so facets can render
    /// names without a second paged request racing the graph snapshot.
    pub tags: Vec<Tag>,
    pub truncated: bool,
    pub node_limit: u32,
    pub edge_limit: u32,
}

fn bounded_title(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return fallback.to_owned();
    }
    value.chars().take(MAX_BLOCK_TITLE_CHARS).collect()
}

fn yaml_field<'a>(mapping: &'a serde_yaml_ng::Mapping, key: &str) -> Option<&'a Value> {
    mapping.iter().find_map(|(candidate, value)| {
        matches!(candidate, Value::String(candidate) if candidate == key).then_some(value)
    })
}

/// Relation fields are extensible YAML, but their stable contract is a list of
/// `{kind: note, id, title}` records. Only contract-shaped records enter the
/// graph; malformed or unrelated frontmatter stays inert and never blocks it.
pub(crate) fn relation_targets(source: &str) -> Vec<(String, String, String)> {
    let Ok(Value::Mapping(mapping)) = serde_yaml_ng::from_str::<Value>(source) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for (key, value) in mapping.iter() {
        let Value::String(key) = key else {
            continue;
        };
        let Value::Sequence(items) = value else {
            continue;
        };
        for item in items {
            let Value::Mapping(entry) = item else {
                continue;
            };
            let Some(Value::String(kind)) = yaml_field(entry, "kind") else {
                continue;
            };
            let Some(Value::String(note_id)) = yaml_field(entry, "id") else {
                continue;
            };
            let Some(Value::String(title)) = yaml_field(entry, "title") else {
                continue;
            };
            let title = title.trim();
            if kind != "note"
                || title.is_empty()
                || title.chars().count() > 300
                || crate::document::valid_uuid(note_id).is_err()
            {
                continue;
            }
            targets.push((note_id.clone(), key.clone(), title.to_owned()));
        }
    }
    targets
}

fn tag_ids(json: String) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str(&json).map_err(|_| {
        rusqlite::Error::InvalidColumnType(0, "tag_ids".into(), rusqlite::types::Type::Text)
    })
}

impl Library {
    /// Builds the bounded live graph. Search, node-type and tag filters remain
    /// presentational, so panning and filtering never wait on SQLite.
    pub fn knowledge_graph(&self) -> Result<KnowledgeGraph> {
        let tx = self.connection.unchecked_transaction()?;
        let mut nodes = BTreeMap::<String, KnowledgeGraphNode>::new();
        let mut truncated = false;

        let mut statement = tx.prepare(
            "SELECT o.kind,o.id,n.title,s.display_title,
                    CASE WHEN trim(coalesce(a.comment,''))='' THEN coalesce(s2.display_title || ' · annotation','Annotation') ELSE substr(a.comment,1,160) END,
                    coalesce((SELECT json_group_array(ot.tag_id) FROM object_tags ot WHERE ot.object_id=o.id),'[]'),
                    coalesce((SELECT json_group_array(m.collection_id) FROM memberships m WHERE m.member_id=o.id),'[]')
             FROM objects o
             LEFT JOIN notes n ON n.id=o.id AND o.kind='note'
             LEFT JOIN sources s ON s.id=o.id AND o.kind='source'
             LEFT JOIN annotations a ON a.id=o.id AND o.kind='annotation'
             LEFT JOIN source_versions av ON av.id=a.source_version_id
             LEFT JOIN sources s2 ON s2.id=av.source_id
             WHERE o.trashed=0 AND o.kind IN ('note','source','annotation')
             ORDER BY CASE o.kind WHEN 'note' THEN 0 WHEN 'source' THEN 1 ELSE 2 END,o.id
             LIMIT ?1",
        )?;
        let object_rows = statement
            .query_map([(MAX_GRAPH_NODES + 1) as i64], |row| {
                let kind: String = row.get(0)?;
                let kind = match kind.as_str() {
                    "note" => KnowledgeGraphNodeKind::Note,
                    "source" => KnowledgeGraphNodeKind::Source,
                    "annotation" => KnowledgeGraphNodeKind::Annotation,
                    _ => {
                        return Err(rusqlite::Error::InvalidColumnType(
                            0,
                            "kind".into(),
                            rusqlite::types::Type::Text,
                        ));
                    }
                };
                let id: String = row.get(1)?;
                let title = match kind {
                    KnowledgeGraphNodeKind::Note => row.get::<_, Option<String>>(2)?,
                    KnowledgeGraphNodeKind::Source => row.get::<_, Option<String>>(3)?,
                    _ => row.get::<_, Option<String>>(4)?,
                }
                .unwrap_or_default();
                Ok(KnowledgeGraphNode {
                    id: id.clone(),
                    title: bounded_title(
                        &title,
                        match kind {
                            KnowledgeGraphNodeKind::Note => "Untitled note",
                            KnowledgeGraphNodeKind::Source => "Untitled source",
                            _ => "Annotation",
                        },
                    ),
                    kind,
                    tag_ids: tag_ids(row.get(5)?)?,
                    collection_ids: tag_ids(row.get(6)?)?,
                    degree: 0,
                    object_id: Some(id),
                    block_id: None,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if object_rows.len() > MAX_GRAPH_NODES {
            truncated = true;
        }
        for node in object_rows.into_iter().take(MAX_GRAPH_NODES) {
            nodes.insert(node.id.clone(), node);
        }

        let remaining = MAX_GRAPH_NODES.saturating_sub(nodes.len());
        let mut block_map = BTreeMap::<(String, String), String>::new();
        if remaining > 0 {
            let mut statement = tx.prepare(
                "SELECT b.id,b.note_id,n.title,b.text,
                        coalesce((SELECT json_group_array(ot.tag_id) FROM object_tags ot WHERE ot.object_id=b.note_id),'[]'),
                        coalesce((SELECT json_group_array(m.collection_id) FROM memberships m WHERE m.member_id=b.note_id),'[]')
                 FROM blocks b JOIN notes n ON n.id=b.note_id
                 WHERE b.active=1 AND n.trashed=0
                   AND EXISTS(SELECT 1 FROM body_links l WHERE l.target_id=b.note_id AND l.target_block_id=b.id)
                 ORDER BY b.note_id,b.position,b.id LIMIT ?1",
            )?;
            let rows = statement
                .query_map([(remaining + 1) as i64], |row| {
                    let block_id: String = row.get(0)?;
                    let note_id: String = row.get(1)?;
                    let note_title: String = row.get(2)?;
                    let text: String = row.get(3)?;
                    let title = bounded_title(&text, &bounded_title(&note_title, "Block"));
                    Ok(KnowledgeGraphNode {
                        id: format!("block:{block_id}"),
                        title,
                        kind: KnowledgeGraphNodeKind::Block,
                        tag_ids: tag_ids(row.get(4)?)?,
                        collection_ids: tag_ids(row.get(5)?)?,
                        degree: 0,
                        object_id: Some(note_id.clone()),
                        block_id: Some(block_id.clone()),
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if rows.len() > remaining {
                truncated = true;
            }
            for node in rows.into_iter().take(remaining) {
                block_map.insert(
                    (
                        node.object_id.clone().unwrap_or_default(),
                        node.block_id.clone().unwrap_or_default(),
                    ),
                    node.id.clone(),
                );
                nodes.insert(node.id.clone(), node);
            }
        }

        let remaining = MAX_GRAPH_NODES.saturating_sub(nodes.len());
        if remaining > 0 {
            let mut statement =
                tx.prepare("SELECT id,name FROM canvases ORDER BY name,id LIMIT ?1")?;
            let rows = statement
                .query_map([(remaining + 1) as i64], |row| {
                    let id: String = row.get(0)?;
                    Ok(KnowledgeGraphNode {
                        id: format!("canvas:{id}"),
                        title: row.get(1)?,
                        kind: KnowledgeGraphNodeKind::Canvas,
                        tag_ids: vec![],
                        collection_ids: vec![],
                        degree: 0,
                        object_id: Some(id),
                        block_id: None,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if rows.len() > remaining {
                truncated = true;
            }
            for node in rows.into_iter().take(remaining) {
                nodes.insert(node.id.clone(), node);
            }
        }

        let mut edges = BTreeMap::<(String, String, String), KnowledgeGraphEdge>::new();
        let mut statement = tx.prepare(
            "SELECT source_id,target_id,target_block_id,COUNT(*),substr(coalesce(min(context),''),1,?2)
             FROM body_links GROUP BY source_id,target_id,target_block_id
             ORDER BY COUNT(*) DESC,source_id,target_id,target_block_id LIMIT ?1",
        )?;
        let body_rows = statement
            .query_map(
                params![(MAX_GRAPH_EDGES + 1) as i64, MAX_EDGE_CONTEXT_CHARS as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if body_rows.len() > MAX_GRAPH_EDGES {
            truncated = true;
        }
        for (source_id, target_object_id, target_block_id, weight, context) in
            body_rows.into_iter().take(MAX_GRAPH_EDGES)
        {
            if !nodes.contains_key(&source_id) {
                continue;
            }
            let block_node_id = target_block_id.as_ref().and_then(|block_id| {
                block_map
                    .get(&(target_object_id.clone(), block_id.clone()))
                    .cloned()
            });
            let (target_id, kind) = match block_node_id {
                Some(id) => (id, KnowledgeGraphEdgeKind::BlockReference),
                None if nodes.contains_key(&target_object_id) => {
                    (target_object_id, KnowledgeGraphEdgeKind::Reference)
                }
                None => continue,
            };
            let key = (source_id.clone(), target_id.clone(), format!("{kind:?}"));
            let edge = edges.entry(key).or_insert_with(|| KnowledgeGraphEdge {
                source_id,
                target_id,
                kind,
                weight: 0,
                context,
                target_block_id: if kind == KnowledgeGraphEdgeKind::BlockReference {
                    target_block_id
                } else {
                    None
                },
            });
            edge.weight = edge.weight.saturating_add(weight);
        }

        let mut statement = tx.prepare(
            "SELECT n.id,json_extract(r.document_json,'$.content[0].attrs.source')
             FROM notes n JOIN note_revisions r ON r.revision_id=n.current_revision_id
             WHERE n.trashed=0
               AND json_extract(r.document_json,'$.content[0].type')='frontmatter'
             ORDER BY n.id LIMIT ?1",
        )?;
        let relation_notes = statement
            .query_map([(MAX_GRAPH_NODES + 1) as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if relation_notes.len() > MAX_GRAPH_NODES {
            truncated = true;
        }
        for (relation_source_id, source) in relation_notes.into_iter().take(MAX_GRAPH_NODES) {
            if !nodes.contains_key(&relation_source_id) {
                continue;
            }
            for (target_id, key, title) in relation_targets(&source) {
                if !nodes.contains_key(&target_id) {
                    continue;
                }
                let context = format!("{key}: {title}")
                    .chars()
                    .take(MAX_EDGE_CONTEXT_CHARS)
                    .collect::<String>();
                let edge = edges
                    .entry((
                        relation_source_id.clone(),
                        target_id.clone(),
                        "Relation".into(),
                    ))
                    .or_insert(KnowledgeGraphEdge {
                        source_id: relation_source_id.clone(),
                        target_id,
                        kind: KnowledgeGraphEdgeKind::Relation,
                        weight: 0,
                        context,
                        target_block_id: None,
                    });
                edge.weight = edge.weight.saturating_add(1);
            }
        }
        let mut statement = tx.prepare(
            "SELECT c.id,json_extract(j.value,'$.note_id'),c.name
             FROM canvases c,json_each(c.nodes_json) j
             WHERE json_type(j.value,'$.note_id')='text'
             GROUP BY c.id,json_extract(j.value,'$.note_id')
             ORDER BY c.name,c.id,json_extract(j.value,'$.note_id') LIMIT ?1",
        )?;
        let canvas_rows = statement
            .query_map([(MAX_GRAPH_EDGES + 1) as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if canvas_rows.len() > MAX_GRAPH_EDGES {
            truncated = true;
        }
        for (canvas_id, note_id, name) in canvas_rows.into_iter().take(MAX_GRAPH_EDGES) {
            let source_id = format!("canvas:{canvas_id}");
            if !nodes.contains_key(&source_id) || !nodes.contains_key(&note_id) {
                continue;
            }
            edges
                .entry((source_id.clone(), note_id.clone(), "Canvas".into()))
                .or_insert(KnowledgeGraphEdge {
                    source_id,
                    target_id: note_id,
                    kind: KnowledgeGraphEdgeKind::Canvas,
                    weight: 1,
                    context: name.chars().take(MAX_EDGE_CONTEXT_CHARS).collect(),
                    target_block_id: None,
                });
        }

        let mut edges = edges.values().cloned().collect::<Vec<_>>();
        edges.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then_with(|| a.source_id.cmp(&b.source_id))
                .then_with(|| a.target_id.cmp(&b.target_id))
        });
        edges.truncate(MAX_GRAPH_EDGES);
        let mut neighbours = BTreeMap::<String, BTreeSet<String>>::new();
        for edge in &edges {
            neighbours
                .entry(edge.source_id.clone())
                .or_default()
                .insert(edge.target_id.clone());
            neighbours
                .entry(edge.target_id.clone())
                .or_default()
                .insert(edge.source_id.clone());
        }
        for (id, values) in neighbours {
            if let Some(node) = nodes.get_mut(&id) {
                node.degree = u32::try_from(values.len()).unwrap_or(u32::MAX);
            }
        }
        let mut nodes = nodes.into_values().collect::<Vec<_>>();
        nodes.sort_by(|a, b| {
            b.degree
                .cmp(&a.degree)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.id.cmp(&b.id))
        });
        let used_tag_ids = nodes
            .iter()
            .flat_map(|node| node.tag_ids.iter())
            .collect::<BTreeSet<_>>();
        let used_tag_ids = serde_json::to_string(&used_tag_ids)?;
        let mut statement = tx.prepare(
            "SELECT id,display_name,normalized_name FROM tags
             WHERE id IN (SELECT value FROM json_each(?1))
             ORDER BY normalized_name,id",
        )?;
        let tags = statement
            .query_map([used_tag_ids], |row| {
                Ok(Tag {
                    id: row.get(0)?,
                    display_name: row.get(1)?,
                    normalized_name: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(KnowledgeGraph {
            nodes,
            edges,
            tags,
            truncated,
            node_limit: MAX_GRAPH_NODES as u32,
            edge_limit: MAX_GRAPH_EDGES as u32,
        })
    }
}

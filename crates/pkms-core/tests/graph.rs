use pkms_core::*;
use serde_json::json;
use tempfile::tempdir;
use uuid::Uuid;

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn reference(target: &Note, occurrence: String) -> serde_json::Value {
    json!({
        "type": "noteReference",
        "attrs": {
            "target_id": target.id,
            "occurrence_id": occurrence,
            "label": target.title
        }
    })
}

#[test]
fn contract_shaped_frontmatter_relations_become_graph_edges_without_body_links() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db.create_note("Related target", "Target body").unwrap();
    let document = serde_json::from_value(json!({
        "type": "doc",
        "content": [
            {
                "type": "frontmatter",
                "attrs": {
                    "id": id(),
                    "source": format!("type: relation-archive\nrelated:\n  - kind: note\n    id: {}\n    title: Related target\nunrelated:\n  - kind: note\n    id: not-a-uuid\n    title: Ignore me\n", target.id)
                }
            },
            { "type": "paragraph", "attrs": { "id": id() } }
        ]
    }))
    .unwrap();
    let source = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "Relation source".into(),
                document_schema: 3,
                document,
            },
        )
        .unwrap()
        .note;

    let graph = db.knowledge_graph().unwrap();
    let relation_edges = graph
        .edges
        .iter()
        .filter(|edge| edge.kind == KnowledgeGraphEdgeKind::Relation)
        .collect::<Vec<_>>();
    assert_eq!(relation_edges.len(), 1);
    assert_eq!(relation_edges[0].source_id, source.id);
    assert_eq!(relation_edges[0].target_id, target.id);
    assert_eq!(relation_edges[0].weight, 1);
    assert!(
        relation_edges[0]
            .context
            .contains("related: Related target")
    );
    assert!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == source.id)
            .is_some_and(|node| node.degree == 1)
    );
}
#[test]
fn live_body_links_become_weighted_edges_and_orphans_remain_visible() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db.create_note("Target", "Target body").unwrap();
    let orphan = db.create_note("Orphan", "No links yet").unwrap();
    let document = serde_json::from_value(json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "attrs": { "id": id() },
                "content": [
                    { "type": "text", "text": "First " },
                    reference(&target, id())
                ]
            },
            {
                "type": "paragraph",
                "attrs": { "id": id() },
                "content": [
                    { "type": "text", "text": "Second " },
                    reference(&target, id())
                ]
            }
        ]
    }))
    .unwrap();
    let source = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "Source".into(),
                document_schema: 2,
                document,
            },
        )
        .unwrap()
        .note;

    let graph = db.knowledge_graph().unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.edges.len(), 1);
    let edge = &graph.edges[0];
    assert_eq!(edge.source_id, source.id);
    assert_eq!(edge.target_id, target.id);
    assert_eq!(edge.weight, 2);
    assert!(!edge.context.is_empty());

    let degrees = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.degree))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(degrees[source.id.as_str()], 1);
    assert_eq!(degrees[target.id.as_str()], 1);
    assert_eq!(degrees[orphan.id.as_str()], 0);
    assert!(!graph.truncated);
}

#[test]
fn graph_projection_carries_the_tag_vocabulary_used_by_its_nodes() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db.create_note("Tagged note", "Tagged body").unwrap();
    let row_version = db.object_metadata(&note.id).unwrap().row_version;
    let assigned = db
        .assign_tag(
            &id(),
            AssignTag {
                targets: vec![MetadataTarget {
                    object_id: note.id.clone(),
                    base_row_version: row_version,
                }],
                tag: TagSelector::Name {
                    name: "Research".into(),
                },
            },
        )
        .unwrap();
    let tag = assigned.tag.unwrap();

    let graph = db.knowledge_graph().unwrap();
    assert!(graph.tags.iter().any(|candidate| candidate.id == tag.id));
    assert!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == note.id)
            .is_some_and(|node| node.tag_ids.contains(&tag.id))
    );
}

#[test]
fn trashed_endpoints_never_leak_into_the_global_graph() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target = db.create_note("Hidden target", "Body").unwrap();
    let document = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": { "id": id() },
            "content": [reference(&target, id())]
        }]
    }))
    .unwrap();
    let source = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "Visible source".into(),
                document_schema: 2,
                document,
            },
        )
        .unwrap()
        .note;
    db.set_trashed(&target.id, true).unwrap();

    let graph = db.knowledge_graph().unwrap();
    assert!(graph.nodes.iter().all(|node| node.id != target.id));
    assert!(graph.nodes.iter().any(|node| node.id == source.id));
    assert!(graph.edges.is_empty());
    assert!(
        graph
            .nodes
            .iter()
            .find(|node| node.id == source.id)
            .is_some_and(|node| node.degree == 0)
    );
}

#[test]
fn block_references_and_canvas_membership_are_typed_graph_relations() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let target_block = id();
    let target_document = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": { "id": target_block, "block_id": "graph-block" },
            "content": [{ "type": "text", "text": "Linked block evidence" }]
        }]
    }))
    .unwrap();
    let target = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "Block target".into(),
                document_schema: 3,
                document: target_document,
            },
        )
        .unwrap()
        .note;
    let source_document = serde_json::from_value(json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": { "id": id() },
            "content": [{
                "type": "noteReference",
                "attrs": {
                    "target_id": target.id,
                    "target_block_id": target_block,
                    "occurrence_id": id(),
                    "label": "Block target"
                }
            }]
        }]
    }))
    .unwrap();
    let source = db
        .create_document(
            &id(),
            CreateNote {
                collection_id: None,
                base_collection_row_version: None,
                title: "Block source".into(),
                document_schema: 3,
                document: source_document,
            },
        )
        .unwrap()
        .note;
    let canvas = db
        .canvas_save(
            &id(),
            SaveCanvas {
                canvas_id: None,
                base_row_version: None,
                name: "Graph canvas".into(),
                config_schema: 1,
                nodes: vec![CanvasNode {
                    note_id: target.id.clone(),
                    x: 0.0,
                    y: 0.0,
                    width: 250.0,
                    height: 148.0,
                    z_index: 0,
                }],
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .canvas;

    let graph = db.knowledge_graph().unwrap();
    let block_id = format!("block:{target_block}");
    let canvas_id = format!("canvas:{}", canvas.id);
    let block = graph.nodes.iter().find(|node| node.id == block_id).unwrap();
    assert_eq!(block.kind, KnowledgeGraphNodeKind::Block);
    assert_eq!(block.object_id.as_deref(), Some(target.id.as_str()));
    assert_eq!(block.block_id.as_deref(), Some(target_block.as_str()));
    let canvas_node = graph
        .nodes
        .iter()
        .find(|node| node.id == canvas_id)
        .unwrap();
    assert_eq!(canvas_node.kind, KnowledgeGraphNodeKind::Canvas);
    assert_eq!(canvas_node.object_id.as_deref(), Some(canvas.id.as_str()));
    assert!(graph.edges.iter().any(|edge| {
        edge.source_id == source.id
            && edge.target_id == block_id
            && edge.kind == KnowledgeGraphEdgeKind::BlockReference
            && edge.target_block_id.as_deref() == Some(target_block.as_str())
    }));
    assert!(graph.edges.iter().any(|edge| {
        edge.source_id == canvas_id
            && edge.target_id == target.id
            && edge.kind == KnowledgeGraphEdgeKind::Canvas
    }));
}

#[test]
fn graph_nodes_carry_collection_membership_for_category_views() {
    let root = tempdir().unwrap();
    let mut db = Library::open(root.path()).unwrap();
    let note = db
        .create_note("同步冲突与来源版本：PDF 第 3 页摘录笔记", "项目复盘附录 B")
        .unwrap();
    let first = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "项目复盘".into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .collection;
    let second = db
        .collection_create(
            &id(),
            CreateCollection {
                name: "同步冲突研究".into(),
                overview_note_id: None,
                is_pinned: false,
                pin_position: None,
            },
        )
        .unwrap()
        .collection;
    for collection in [&first, &second] {
        db.collection_add_members(
            &id(),
            ChangeCollectionMembers {
                collection_id: collection.id.clone(),
                base_row_version: collection.metadata.row_version,
                member_ids: vec![note.id.clone()],
            },
        )
        .unwrap();
    }
    let graph = db.knowledge_graph().unwrap();
    let node = graph.nodes.iter().find(|node| node.id == note.id).unwrap();
    let mut collections = node.collection_ids.clone();
    collections.sort();
    let mut expected = vec![first.id.clone(), second.id.clone()];
    expected.sort();
    assert_eq!(collections, expected);
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == KnowledgeGraphNodeKind::Canvas)
            .all(|node| node.collection_ids.is_empty())
    );
}

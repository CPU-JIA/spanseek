//! Reproducible 10k-node knowledge graph fixture and warm projection probe.
//! Every note is created through the public app/core API in a new isolated
//! workspace. No database rows are written directly and no real library is read.
use pkms_app::{AppService, GrantPurpose, OperationKind};
use pkms_core::CreateNote;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn summary(mut samples: Vec<f64>) -> Value {
    samples.sort_by(f64::total_cmp);
    json!({
        "n": samples.len(),
        "p50_ms": samples[(samples.len() * 50).div_ceil(100) - 1],
        "p95_ms": samples[(samples.len() * 95).div_ceil(100) - 1],
        "max_ms": samples.last(),
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "probe" {
        let root = PathBuf::from(&args[2]);
        let mut app = AppService::open(root.join("app"))?;
        let grant = app.grant_path(GrantPurpose::OpenWorkspace, root.join("knowledge"))?;
        let operation =
            app.accept_workspace_operation(&id(), OperationKind::OpenWorkspace, &grant.token)?;
        let identity = app
            .execute_operation(&operation.operation_id)?
            .result
            .ok_or("workspace open failed")?
            .identity;
        let mut samples = Vec::new();
        let mut observed = None;
        for _ in 0..11 {
            let started = Instant::now();
            let graph = app.library(&identity)?.knowledge_graph()?;
            samples.push(ms(started));
            observed = Some((graph.nodes.len(), graph.edges.len(), graph.truncated));
        }
        let (nodes, edges, truncated) = observed.ok_or("no graph sample")?;
        samples.remove(0);
        let result = json!({
            "synthetic": true,
            "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            "root": root,
            "identity": identity,
            "nodes": nodes,
            "edges": edges,
            "truncated": truncated,
            "graph_projection": summary(samples),
        });
        fs::write(root.join("probe.json"), serde_json::to_vec_pretty(&result)?)?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if args.len() != 3 {
        return Err(
            "usage: graph_scale_fixture <new-root> <nodes=10000> | probe <existing-root>".into(),
        );
    }
    let root = PathBuf::from(&args[1]);
    let count: usize = args[2].parse()?;
    if root.exists() || !(100..=10_000).contains(&count) {
        return Err("new directory and 100..10000 nodes required".into());
    }
    fs::create_dir_all(&root)?;
    let mut app = AppService::open(root.join("app"))?;
    let grant = app.grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))?;
    let operation =
        app.accept_workspace_operation(&id(), OperationKind::CreateWorkspace, &grant.token)?;
    let identity = app
        .execute_operation(&operation.operation_id)?
        .result
        .ok_or("workspace creation failed")?
        .identity;

    let creation_started = Instant::now();
    let mut previous: Option<(String, String)> = None;
    for index in 0..count {
        let title = format!("Graph node {index:05}");
        let mut content = vec![json!({
            "type": "paragraph",
            "attrs": {"id": id()},
            "content": [{"type": "text", "text": format!("Synthetic graph node {index:05}. ")}],
        })];
        if let Some((target_id, label)) = &previous
            && let Some(paragraph) = content[0].as_object_mut()
        {
            paragraph
                .entry("content")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("paragraph content")
                .push(json!({
                    "type": "noteReference",
                    "attrs": {
                        "target_id": target_id,
                        "occurrence_id": id(),
                        "label": label,
                    },
                }));
        }
        let note = app
            .library_mut(&identity)?
            .create_document(
                &id(),
                CreateNote {
                    title: title.clone(),
                    document_schema: 3,
                    document: serde_json::from_value(json!({"type":"doc","content":content}))?,
                    collection_id: None,
                    base_collection_row_version: None,
                },
            )?
            .note;
        previous = Some((note.id, note.title));
        if (index + 1) % 500 == 0 {
            eprintln!(
                "created {}/{count} notes ({:.1}s)",
                index + 1,
                creation_started.elapsed().as_secs_f64()
            );
        }
    }
    let creation_ms = ms(creation_started);

    let graph_started = Instant::now();
    let graph = app.library(&identity)?.knowledge_graph()?;
    let graph_ms = ms(graph_started);
    if graph.nodes.len() != count {
        return Err(format!("expected {count} graph nodes, got {}", graph.nodes.len()).into());
    }
    if graph.edges.len() < count.saturating_sub(1) {
        return Err(format!(
            "expected at least {} edges, got {}",
            count - 1,
            graph.edges.len()
        )
        .into());
    }

    let result = json!({
        "synthetic": true,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "root": root,
        "identity": identity,
        "nodes": graph.nodes.len(),
        "edges": graph.edges.len(),
        "truncated": graph.truncated,
        "node_limit": graph.node_limit,
        "edge_limit": graph.edge_limit,
        "creation_ms": creation_ms,
        "graph_projection_ms": graph_ms,
        "sample_node_ids": graph.nodes.iter().take(10).map(|node| &node.id).collect::<Vec<_>>(),
    });
    fs::write(
        root.join("dataset.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

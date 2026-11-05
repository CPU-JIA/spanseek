//! Explicit local diagnostic: measures real core and vector stages without publishing.
#[cfg(feature = "vector")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pkms_core::{Library, SearchMode, SearchQuery, SearchScope};
    use pkms_inference::vector::VectorProjection;
    use std::{path::Path, time::Instant};
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: search_stage_probe <library> <vector-directory>".into());
    }
    let library = Library::open_existing(Path::new(&args[1]))?;
    let profile = library.active_embedding_profile()?.ok_or("no profile")?;
    let query = SearchQuery {
        query: "知识管理".into(),
        mode: SearchMode::Hybrid,
        scope: SearchScope::default(),
        limit: 30,
        minimum_cosine: None,
    };
    for sample in 0..5 {
        let start = Instant::now();
        library.keyword_search(&query)?;
        let lexical_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let generations = library.active_index_generations_in_scope(&query.scope)?;
        let scope_ms = start.elapsed().as_secs_f64() * 1000.;
        let expected = generations
            .iter()
            .map(|g| (g.id.clone(), g.chunk_count))
            .collect::<Vec<_>>();
        let ids = generations.iter().map(|g| g.id.clone()).collect::<Vec<_>>();
        let start = Instant::now();
        let projection = VectorProjection::open(Path::new(&args[2])).await?;
        let open_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        projection
            .verify_generations(&profile.id, &expected)
            .await?;
        let verify_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        // Fixed normalized direction isolates storage latency; not a retrieval quality test.
        let vectors = projection
            .search(
                &profile.id,
                &ids,
                &vec![1.; profile.spec.dimensions as usize],
                240,
            )
            .await?;
        let vector_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let result = library.search_with_annotation_vectors(&query, vectors, Vec::new())?;
        let fusion_ms = start.elapsed().as_secs_f64() * 1000.;
        println!(
            "{}",
            serde_json::json!({"sample":sample,"generations":ids.len(),"lexical_ms":lexical_ms,"scope_ms":scope_ms,"open_ms":open_ms,"verify_ms":verify_ms,"vector_ms":vector_ms,"fusion_ms":fusion_ms,"results":result.items.len()})
        );
    }
    Ok(())
}
#[cfg(not(feature = "vector"))]
fn main() {
    eprintln!("enable --features vector");
    std::process::exit(1);
}

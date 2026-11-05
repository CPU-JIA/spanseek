//! Read-only synthetic-library diagnostic. No remote request or index publication.
#[cfg(feature = "vector")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use pkms_core::{Library, SearchScope};
    use pkms_inference::vector::{VectorConnectionCache, VectorProjection};
    use std::{path::Path, time::Instant};
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: vector_connection_probe <library> <vectors>".into());
    }
    let library = Library::open_existing(Path::new(&args[1]))?;
    let profile = library.active_embedding_profile()?.ok_or("no profile")?;
    let generations = library.active_index_generations_in_scope(&SearchScope::default())?;
    let expected = generations
        .iter()
        .map(|g| (g.id.clone(), g.chunk_count))
        .collect::<Vec<_>>();
    let ids = generations.iter().map(|g| g.id.clone()).collect::<Vec<_>>();
    drop(library);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut cache = VectorConnectionCache::default();
        for pair in 0..6 {
            let mut previous = None;
            for pooled in if pair % 2 == 0 { [false,true] } else { [true,false] } {
                let start = Instant::now();
                let fresh;
                let projection = if pooled { cache.get(Path::new(&args[2])).await? } else {
                    fresh = VectorProjection::open(Path::new(&args[2])).await?; &fresh
                };
                let open_ms=start.elapsed().as_secs_f64()*1000.;
                let phase=Instant::now();
                projection.verify_generations(&profile.id,&expected).await?;
                let verify_ms=phase.elapsed().as_secs_f64()*1000.;
                let phase=Instant::now();
                let items=projection.search(&profile.id,&ids,&vec![1.;profile.spec.dimensions as usize],240).await?;
                let search_ms=phase.elapsed().as_secs_f64()*1000.;
                let total_ms=start.elapsed().as_secs_f64()*1000.;
                let bytes=serde_json::to_vec(&items)?;
                if let Some(old)=previous.as_ref() { assert_eq!(old,&bytes,"connection reuse changed candidates"); }
                previous=Some(bytes);
                println!("{}",serde_json::json!({"pair":pair,"pooled":pooled,"generations":ids.len(),"candidates":items.len(),"open_ms":open_ms,"verify_ms":verify_ms,"search_ms":search_ms,"total_ms":total_ms}));
            }
        }
        Ok::<(),Box<dyn std::error::Error>>(())
    })
}
#[cfg(not(feature = "vector"))]
fn main() {
    eprintln!("enable --features vector");
    std::process::exit(1);
}

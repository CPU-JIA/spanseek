//! Explicit derived-index experiment on an authorized synthetic library only.
#[cfg(feature = "vector")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use arrow_array::{Array, FixedSizeListArray, Float32Array, StringArray};
    use futures::TryStreamExt;
    use lancedb::{
        DistanceType,
        expr::{col, lit},
        index::{Index, vector::IvfFlatIndexBuilder},
        query::{ExecutableQuery, QueryBase, Select},
    };
    use pkms_core::{Library, SearchScope};
    use sha2::{Digest, Sha256};
    use std::{path::Path, time::Instant};
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 || args[3] != "--build-synthetic-ivf" {
        return Err(
            "usage: vector_acceleration_probe <synthetic-library> <vectors> --build-synthetic-ivf"
                .into(),
        );
    }
    let library = Library::open_existing(&args[1])?;
    let profile = library.active_embedding_profile()?.ok_or("no profile")?;
    let generations = library.active_index_generations_in_scope(&SearchScope::default())?;
    drop(library);
    let mut hash = Sha256::new();
    hash.update((profile.id.len() as u64).to_le_bytes());
    hash.update(profile.id.as_bytes());
    let name = format!("profile_{:x}", hash.finalize());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let connection=lancedb::connect(Path::new(&args[2]).to_str().ok_or("invalid path")?).read_consistency_interval(std::time::Duration::ZERO).execute().await?;
        let table=connection.open_table(name).execute().await?;
        if !table.list_indices().await?.is_empty() { return Err("preserve pre-existing accelerator; use a fresh diagnostic".into()); }
        let filter=col("generation_id").in_list(generations.iter().map(|g|lit(g.id.clone())).collect(),false);
        let batches=table.query().only_if_expr(filter.clone()).select(Select::columns(&["vector"])).limit(300).execute().await?.try_collect::<Vec<_>>().await?;
        let mut vectors=Vec::new(); let mut row_index=0;
        for batch in batches {
            let column=batch.column_by_name("vector").and_then(|a|a.as_any().downcast_ref::<FixedSizeListArray>()).ok_or("invalid vector array")?;
            for i in 0..batch.num_rows() {
                if row_index % 12 == 0 {
                    let vector=column.value(i);let values=vector.as_any().downcast_ref::<Float32Array>().ok_or("invalid vector")?;
                    vectors.push(values.values().to_vec());
                }
                row_index+=1;
            }
        }
        vectors.truncate(20); if vectors.len()!=20 { return Err("expected a scale fixture".into()); }
        let extract=|batches: Vec<arrow_array::RecordBatch>| -> Result<Vec<(String,f32)>,Box<dyn std::error::Error>> {
            let mut rows=Vec::new();
            for batch in batches {
                let ids=batch.column_by_name("chunk_id").and_then(|a|a.as_any().downcast_ref::<StringArray>()).ok_or("missing IDs")?;
                let distance=batch.column_by_name("_distance").and_then(|a|a.as_any().downcast_ref::<Float32Array>()).ok_or("missing distance")?;
                for i in 0..batch.num_rows() { rows.push((ids.value(i).to_owned(),distance.value(i))); }
            }
            Ok(rows)
        };
        let mut truth=Vec::new();
        for (q,vector) in vectors.iter().enumerate() {
            let start=Instant::now();
            let result=table.query().only_if_expr(filter.clone()).nearest_to(vector.as_slice())?.distance_type(DistanceType::Cosine).bypass_vector_index().select(Select::columns(&["chunk_id","_distance"])).limit(240).execute().await?.try_collect::<Vec<_>>().await?;
            let rows=extract(result)?;
            println!("{}",serde_json::json!({"stage":"exact","query":q,"ms":start.elapsed().as_secs_f64()*1000.,"count":rows.len()}));
            truth.push(rows);
        }
        let started=Instant::now();
        table.create_index(&["vector"],Index::IvfFlat(IvfFlatIndexBuilder::default().distance_type(DistanceType::Cosine).num_partitions(64))).execute().await?;
        println!("{}",serde_json::json!({"stage":"build","ms":started.elapsed().as_secs_f64()*1000.}));
        for nprobes in [8,16,32,64] {
            for (q,vector) in vectors.iter().enumerate() {
                let start=Instant::now();
                let result=table.query().only_if_expr(filter.clone()).nearest_to(vector.as_slice())?.distance_type(DistanceType::Cosine).nprobes(nprobes).select(Select::columns(&["chunk_id","_distance"])).limit(240).execute().await?.try_collect::<Vec<_>>().await?;
                let rows=extract(result)?;
                let ms=start.elapsed().as_secs_f64()*1000.;
                let threshold=truth[q].last().ok_or("empty truth")?.1;
                let recall=rows.iter().filter(|r|truth[q].iter().any(|t|t.0==r.0)).count() as f64 / truth[q].len() as f64;
                let equal_or_better=rows.iter().filter(|r|r.1<=threshold+1e-5).count();
                let distances_match=rows.iter().all(|r|truth[q].iter().find(|t|t.0==r.0).is_none_or(|t|(t.1-r.1).abs()<1e-5));
                println!("{}",serde_json::json!({"stage":"indexed","query":q,"nprobes":nprobes,"ms":ms,"count":rows.len(),"recall240":recall,"equal_or_better_than_truth_cutoff":equal_or_better,"distances_match":distances_match}));
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

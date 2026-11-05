use clap::{Parser, Subcommand};
use pkms_core::{
    ApplyChangeItem, AssignTag, AuditPageRequest, CancelResearch, ChangeSetKind,
    ChangeSetPageRequest, CreateChangeSet, CreateResearchTask, Library, MetadataTarget,
    RejectChangeSet, ResearchBudget, ResearchPageRequest, ResearchStopConditions,
    SetReviewInsightSettings, TagSelector, TagsPageRequest, UndoAuditEvent, WorkspaceLease,
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "spanseek local knowledge workspace CLI", version)]
struct Arguments {
    #[arg(long)]
    workspace: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
    /// Show stable workspace identity, database version and applied migrations.
    Info,
    Create {
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        body: String,
    },
    Get {
        id: String,
    },
    List,
    Save {
        id: String,
        #[arg(long)]
        base_revision: u32,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
    },
    History {
        id: String,
    },
    RestoreRevision {
        id: String,
        #[arg(long)]
        base_revision: u32,
        #[arg(long)]
        revision: u32,
    },
    Search {
        query: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    CollectionCreate {
        name: String,
    },
    CollectionAdd {
        collection: String,
        note: String,
    },
    CollectionRemove {
        collection: String,
        note: String,
    },
    CollectionList {
        collection: String,
    },
    Link {
        source: String,
        target: String,
        #[arg(long, default_value = "")]
        context: String,
    },
    Backlinks {
        target: String,
    },
    Trash {
        id: String,
    },
    Restore {
        id: String,
    },
    Backup {
        destination: PathBuf,
    },
    /// Export the current workspace as a mobile/desktop resume handoff JSON.
    HandoffExport {
        destination: PathBuf,
    },
    /// Import a resume handoff into the current workspace without overwriting.
    HandoffImport {
        source: PathBuf,
    },
    /// Export active notes as an Obsidian-compatible Markdown vault.
    VaultExport {
        destination: PathBuf,
    },
    /// Import Markdown/Obsidian notes, frontmatter tags and resolvable wikilinks.
    VaultImport {
        source: PathBuf,
    },
    /// Import a BibTeX file as reference notes keyed by citation key.
    BibtexImport {
        source: PathBuf,
    },
    /// Resolve the `[@key]` markers of one note into a reference list.
    Bibliography {
        note: String,
    },
    /// Append a `[@key]` citation marker to a note.
    CitationInsert {
        note: String,
        key: String,
    },
    TagAssign {
        note: String,
        name: String,
    },
    TagList,
    NoteMetadata {
        note: String,
    },
    /// Build a reviewable tidy-up proposal without writing knowledge.
    AutomationPropose {
        /// tag_suggestions, duplicate_candidates, relation_suggestions or stale_sources
        kind: String,
        #[arg(long, default_value_t = 20)]
        limit: u32,
        #[arg(long)]
        stale_after_days: Option<u32>,
    },
    AutomationList {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    AutomationGet {
        change: String,
    },
    AutomationApply {
        change: String,
        item: String,
        #[arg(long)]
        confirm: bool,
    },
    AutomationSkip {
        change: String,
        item: String,
    },
    AutomationReject {
        change: String,
    },
    AutomationExpire,
    StaleReport {
        #[arg(long, default_value_t = 365)]
        window_days: u32,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    AuditList {
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    AuditUndo {
        event: String,
        #[arg(long)]
        confirm: bool,
    },
    ResearchCreate {
        #[arg(long)]
        goal: String,
        #[arg(long = "query", required = true)]
        query: Vec<String>,
        #[arg(long, default_value_t = 5)]
        max_steps: u32,
        #[arg(long, default_value_t = 10)]
        max_sources: u32,
        #[arg(long)]
        external_endpoint: Option<String>,
    },
    ResearchList {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    ResearchGet {
        task: String,
    },
    ResearchRun {
        task: String,
    },
    ResearchCancel {
        task: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
    ReviewInsights {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    ReviewInsightsEnable {
        #[arg(long)]
        enabled: bool,
        #[arg(long, default_value_t = 30)]
        window_days: u32,
    },
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn parse_kind(value: &str) -> std::result::Result<ChangeSetKind, Box<dyn std::error::Error>> {
    Ok(match value {
        "tag_suggestions" => ChangeSetKind::TagSuggestions,
        "duplicate_candidates" => ChangeSetKind::DuplicateCandidates,
        "relation_suggestions" => ChangeSetKind::RelationSuggestions,
        "stale_sources" => ChangeSetKind::StaleSources,
        _ => return Err("unknown change set kind".into()),
    })
}
fn output(value: impl Serialize) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Arguments::parse();
    if matches!(args.command, Command::Init) {
        std::fs::create_dir_all(&args.workspace)?;
    }
    let _owner = WorkspaceLease::acquire(&args.workspace)?;
    let mut library = if matches!(args.command, Command::Init) {
        Library::open(&args.workspace)?
    } else {
        Library::open_existing(&args.workspace)?
    };
    match args.command {
        Command::Init => output(serde_json::json!({"status":"ready","workspace":args.workspace})),
        Command::Create { title, body } => output(library.create_note(&title, &body)?),
        Command::Info => output(library.info()?),
        Command::Get { id } => output(library.note(&id)?),
        Command::List => output(library.list_notes()?),
        Command::Save {
            id,
            base_revision,
            title,
            body,
        } => output(library.save_note(&id, base_revision, &title, &body)?),
        Command::History { id } => output(library.revisions(&id)?),
        Command::RestoreRevision {
            id,
            base_revision,
            revision,
        } => output(library.restore_revision(&id, base_revision, revision)?),
        Command::Search { query, limit } => output(library.search(&query, limit)?),
        Command::CollectionCreate { name } => output(library.create_collection(&name)?),
        Command::CollectionList { collection } => output(library.collection_notes(&collection)?),
        Command::Backlinks { target } => output(library.backlinks(&target)?),
        Command::CollectionAdd { collection, note } => {
            library.add_to_collection(&collection, &note)?;
            output(serde_json::json!({"status":"added"}))
        }
        Command::CollectionRemove { collection, note } => {
            library.remove_from_collection(&collection, &note)?;
            output(serde_json::json!({"status":"removed"}))
        }
        Command::Link {
            source,
            target,
            context,
        } => {
            library.link_notes(&source, &target, &context)?;
            output(serde_json::json!({"status":"linked"}))
        }
        Command::Trash { id } => {
            library.set_trashed(&id, true)?;
            output(serde_json::json!({"status":"trashed"}))
        }
        Command::Restore { id } => {
            library.set_trashed(&id, false)?;
            output(serde_json::json!({"status":"restored"}))
        }
        Command::Backup { destination } => {
            library.backup_to(destination)?;
            output(
                serde_json::json!({"status":"backup_complete","scope":"workspace_database_only"}),
            )
        }
        Command::HandoffExport { destination } => {
            let handoff = library.export_resume_handoff(&[], &HashMap::new())?;
            let bytes = serde_json::to_vec_pretty(&handoff)?;
            let parent = destination
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            std::fs::create_dir_all(parent)?;
            let temporary = destination.with_extension("json.tmp");
            std::fs::write(&temporary, bytes)?;
            std::fs::rename(&temporary, &destination)?;
            output(
                serde_json::json!({"status":"handoff_exported","objects":handoff.objects.len(),"workspace_id":handoff.workspace_id}),
            )
        }
        Command::VaultExport { destination } => {
            output(library.export_obsidian_vault(&destination)?)
        }
        Command::VaultImport { source } => {
            let is_jex = source
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jex"));
            if is_jex {
                output(library.import_joplin_jex(&source)?)
            } else {
                let prefix = format!("vault-import-{}", std::process::id());
                output(library.import_obsidian_vault(&source, &prefix)?)
            }
        }
        Command::BibtexImport { source } => output(library.import_bibliography(&source)?),
        Command::Bibliography { note } => output(library.bibliography(&note)?),
        Command::CitationInsert { note, key } => {
            let updated = library.citation_insert(&note, &key)?;
            output(serde_json::json!({
                "status": "citation_inserted",
                "note_id": updated.id,
                "revision": updated.revision,
                "key": key,
            }))
        }
        Command::TagAssign { note, name } => {
            let metadata = library.object_metadata(&note)?;
            output(library.assign_tag(
                &new_id(),
                AssignTag {
                    targets: vec![MetadataTarget {
                        object_id: note,
                        base_row_version: metadata.row_version,
                    }],
                    tag: TagSelector::Name { name },
                },
            )?)
        }
        Command::NoteMetadata { note } => output(library.object_metadata(&note)?),
        Command::TagList => output(library.tags_page(TagsPageRequest {
            limit: 100,
            cursor: None,
            query: None,
        })?),
        Command::AutomationPropose {
            kind,
            limit,
            stale_after_days,
        } => output(
            library
                .create_change_set(
                    &new_id(),
                    CreateChangeSet {
                        kind: parse_kind(&kind)?,
                        collection_id: None,
                        limit,
                        stale_after_days,
                    },
                )?
                .change,
        ),
        Command::AutomationList { limit } => {
            output(library.change_sets_page(ChangeSetPageRequest {
                limit,
                before_id: None,
                kind: None,
                status: None,
            })?)
        }
        Command::AutomationGet { change } => output(library.change_set(&change)?),
        Command::AutomationApply {
            change,
            item,
            confirm,
        } => output(library.apply_change_item(
            &new_id(),
            ApplyChangeItem {
                change_id: change,
                item_id: item,
                confirm,
            },
        )?),
        Command::AutomationSkip { change, item } => output(library.skip_change_item(
            &new_id(),
            ApplyChangeItem {
                change_id: change,
                item_id: item,
                confirm: false,
            },
        )?),
        Command::AutomationReject { change } => {
            output(library.reject_change_set(&new_id(), RejectChangeSet { change_id: change })?)
        }
        Command::AutomationExpire => output(serde_json::json!({
            "expired": library.expire_change_sets_before(
                chrono::Utc::now() - chrono::Duration::days(7)
            )?,
        })),
        Command::StaleReport { window_days, limit } => {
            output(library.stale_source_report(window_days, limit)?)
        }
        Command::AuditList { limit } => output(library.audit_page(AuditPageRequest {
            limit,
            before_seq: None,
            subject_kind: None,
            subject_id: None,
        })?),
        Command::AuditUndo { event, confirm } => output(library.undo_audit_event(
            &new_id(),
            UndoAuditEvent {
                audit_id: event,
                confirm,
            },
        )?),
        Command::ResearchCreate {
            goal,
            query,
            max_steps,
            max_sources,
            external_endpoint,
        } => output(
            library
                .create_research_task(
                    &new_id(),
                    CreateResearchTask {
                        goal,
                        collection_id: None,
                        queries: query,
                        budget: ResearchBudget {
                            max_steps,
                            max_sources,
                        },
                        stop: ResearchStopConditions {
                            deadline_seconds: None,
                            missing_after_step: None,
                        },
                        external_allowed: external_endpoint.is_some(),
                        external_search_endpoint: external_endpoint,
                    },
                )?
                .task,
        ),
        Command::ResearchList { limit } => output(library.research_page(ResearchPageRequest {
            limit,
            before_id: None,
        })?),
        Command::ResearchGet { task } => output(library.research_task(&task)?),
        Command::ResearchRun { task } => {
            let current = library.research_task(&task)?;
            output(library.run_research_steps(&current.id, current.budget.max_steps)?)
        }
        Command::ResearchCancel { task, reason } => {
            let current = library.research_task(&task)?;
            output(
                library
                    .cancel_research(
                        &new_id(),
                        CancelResearch {
                            task_id: current.id,
                            base_row_version: current.row_version,
                            reason,
                        },
                    )?
                    .task,
            )
        }
        Command::ReviewInsights { limit } => output(library.review_insights(limit)?),
        Command::ReviewInsightsEnable {
            enabled,
            window_days,
        } => {
            let current = library.review_insight_settings()?;
            output(library.set_review_insight_settings(
                &new_id(),
                SetReviewInsightSettings {
                    base_row_version: current.row_version,
                    enabled,
                    window_days,
                    include_completed: true,
                    include_stale: true,
                },
            )?)
        }
        Command::HandoffImport { source } => {
            let bytes = std::fs::read(&source)?;
            let handoff = serde_json::from_slice(&bytes)?;
            let imported = library.import_resume_handoff(&handoff)?;
            output(
                serde_json::json!({"status":"handoff_imported","imported":imported.imported.len(),"skipped_existing":imported.skipped_existing}),
            )
        }
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

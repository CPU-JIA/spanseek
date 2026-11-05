//! Opt-in strict-evidence App Agent probe, not a test or restriction of all AI modes.
//! Only this executable's synthetic workspace is used.
//! Pass one bounded JSON line {profile,api_key} on stdin; never pass a key in argv.
use pkms_app::{
    AgentProgress, AgentStage, AppService, GrantPurpose, InferenceConfig, OperationKind,
    WorkspaceIdentity,
    inference_credentials::{self as vault, CredentialBinding, Secret},
};
use pkms_core::{
    AgentOptions, AnswerRecord, AnswerStatus, GenerationProfile, SearchMode, SearchQuery,
    SearchScope, SetGenerationProfile,
};
use serde::{Deserialize, Serialize};
use std::{
    io::BufRead,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const INPUT_LIMIT: usize = 32 * 1024;
const QUESTION: &str = "请使用知识库工具检索并读取松柏计划的晨间和晚间批次记录，分别回答两种批次的备份保留期限是多少天。用阿拉伯数字并为每种批次标注来源。";
const FACTS: [(&str, &str, &str, &str); 2] = [
    (
        "松柏计划晨间批次",
        "松柏计划晨间批次的备份保留期限为7天。",
        "晨间",
        "7",
    ),
    (
        "松柏计划晚间批次",
        "松柏计划晚间批次的备份保留期限为13天。",
        "晚间",
        "13",
    ),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    profile: GenerationProfile,
    api_key: String,
}

#[derive(Serialize)]
struct Report {
    code: &'static str,
    model_calls: u32,
    tool_calls: u32,
    search_stages: u32,
    read_stages: u32,
    evidence_count: u32,
    credential_removed: bool,
}
impl Default for Report {
    fn default() -> Self {
        Self {
            code: "probe_not_completed",
            model_calls: 0,
            tool_calls: 0,
            search_stages: 0,
            read_stages: 0,
            evidence_count: 0,
            credential_removed: false,
        }
    }
}

fn read_input(reader: impl BufRead) -> Result<Input, &'static str> {
    let mut bytes = Vec::new();
    let read = reader
        .take((INPUT_LIMIT + 1) as u64)
        .read_until(b'\n', &mut bytes);
    let result = if read.is_err() {
        Err("input_failed")
    } else if bytes.len() > INPUT_LIMIT {
        Err("input_too_large")
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "input_invalid")
    };
    bytes.fill(0);
    result
}

struct Setup {
    app: AppService,
    identity: WorkspaceIdentity,
    note_ids: Vec<String>,
    answer_id: String,
}
fn setup(root: &Path, profile: GenerationProfile) -> Result<Setup, &'static str> {
    pkms_core::validate_generation_profile(&profile).map_err(|_| "invalid_profile")?;
    if !matches!(
        profile.provider.as_str(),
        "openai_chat" | "openai_responses" | "anthropic"
    ) {
        return Err("remote_profile_required");
    }
    let mut app = AppService::open(root.join("app")).map_err(|_| "setup_failed")?;
    let grant = app
        .grant_path(GrantPurpose::CreateWorkspace, root.join("knowledge"))
        .map_err(|_| "setup_failed")?;
    let accepted = app
        .accept_workspace_operation(
            &uuid::Uuid::new_v4().to_string(),
            OperationKind::CreateWorkspace,
            &grant.token,
        )
        .map_err(|_| "setup_failed")?;
    let identity = app
        .execute_operation(&accepted.operation_id)
        .map_err(|_| "setup_failed")?
        .result
        .ok_or("setup_failed")?
        .identity;
    let library = app.library_mut(&identity).map_err(|_| "setup_failed")?;
    let mut note_ids = Vec::new();
    for (title, text, _, _) in FACTS {
        note_ids.push(
            library
                .create_note(title, text)
                .map_err(|_| "setup_failed")?
                .id,
        );
    }
    library
        .set_generation_profile(
            &uuid::Uuid::new_v4().to_string(),
            SetGenerationProfile {
                base_row_version: 0,
                profile: Some(profile),
            },
        )
        .map_err(|_| "setup_failed")?;
    let accepted = library
        .accept_agent_answer_from_settings(
            &uuid::Uuid::new_v4().to_string(),
            SearchQuery {
                query: QUESTION.into(),
                mode: SearchMode::Lexical,
                scope: SearchScope {
                    object_ids: note_ids.clone(),
                    ..Default::default()
                },
                limit: 8,
                minimum_cosine: None,
            },
            AgentOptions::default(),
        )
        .map_err(|_| "setup_failed")?;
    Ok(Setup {
        app,
        identity,
        note_ids,
        answer_id: accepted.answer.id,
    })
}

fn config(root: &Path) -> InferenceConfig {
    InferenceConfig {
        executable: root.join("unused-lexical-worker.exe"),
        model_directory: root.join("unused-models"),
        vector_root: root.join("unused-vectors"),
        shutdown: Arc::new(AtomicBool::new(false)),
    }
}

fn record_progress(report: &mut Report, progress: &AgentProgress) {
    report.model_calls = report.model_calls.max(progress.model_calls);
    report.tool_calls = report.tool_calls.max(progress.tool_calls);
    report.evidence_count = report.evidence_count.max(progress.evidence_count);
    match progress.stage {
        AgentStage::Searching => report.search_stages += 1,
        AgentStage::Reading => report.read_stages += 1,
        _ => {}
    }
}

fn verify_answer(
    answer: &AnswerRecord,
    notes: &[String],
    report: &Report,
) -> Result<(), &'static str> {
    if answer.status != AnswerStatus::Completed {
        return Err(match answer.status {
            AnswerStatus::Canceled => "agent_canceled",
            AnswerStatus::TimedOut => "agent_timed_out",
            AnswerStatus::NoEvidence => "agent_no_evidence",
            AnswerStatus::EvidenceChanged => "agent_evidence_changed",
            AnswerStatus::ModelUnconfigured => "agent_model_unconfigured",
            _ => "agent_not_completed",
        });
    }
    if report.tool_calls == 0
        || report.search_stages == 0
        || report.model_calls < 2
        || report.model_calls > 4
        || report.tool_calls > 8
    {
        return Err("agent_tool_loop_not_verified");
    }
    if !answer.evidence_current || answer.evidence.len() != 2 {
        return Err("agent_evidence_not_verified");
    }
    for (index, (_, _, label, days)) in FACTS.iter().enumerate() {
        let evidence = answer
            .evidence
            .iter()
            .find(|item| item.evidence.target.object_id == notes[index])
            .ok_or("agent_evidence_not_verified")?;
        if !answer.cited_ids.contains(&evidence.citation_id) {
            return Err("agent_citations_not_verified");
        }
        let citation = format!("[{}]", evidence.citation_id);
        let other_label = FACTS[1 - index].2;
        let other_days = FACTS[1 - index].3;
        let correct = answer.text.lines().any(|line| {
            line.contains(label)
                && !line.contains(other_label)
                && line.contains(&citation)
                && line
                    .split(|c: char| !c.is_ascii_digit())
                    .any(|number| number == *days)
                && !line
                    .split(|c: char| !c.is_ascii_digit())
                    .any(|number| number == other_days)
        });
        if !correct {
            return Err("synthetic_facts_not_verified");
        }
    }
    Ok(())
}

struct Cleanup(CredentialBinding);
impl Cleanup {
    fn checked(&self) -> Result<(), &'static str> {
        vault::delete(&self.0).map_err(|_| "credential_cleanup_failed")?;
        if vault::exists(&self.0).map_err(|_| "credential_cleanup_failed")? {
            return Err("credential_cleanup_failed");
        }
        Ok(())
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = vault::delete(&self.0);
    }
}

fn probe(input: Input, report: &mut Report) -> Result<(), &'static str> {
    let secret = Secret::new(input.api_key).map_err(|_| "invalid_key")?;
    // Prevent accidentally copying a supplied credential into the persisted profile.
    if [
        &input.profile.provider,
        &input.profile.endpoint,
        &input.profile.model,
        &input.profile.model_digest,
    ]
    .iter()
    .any(|value| value.contains(secret.expose_secret()))
    {
        return Err("credential_in_profile");
    }
    let root = tempfile::Builder::new()
        .prefix("pkms-agent-probe-")
        .tempdir()
        .map_err(|_| "setup_failed")?;
    let fixture = setup(root.path(), input.profile.clone())?;
    let binding = CredentialBinding::new(
        &fixture.identity.local_instance_id,
        &fixture.identity.workspace_id,
        &input.profile.provider,
        &input.profile.endpoint,
        &input.profile.model,
    )
    .map_err(|_| "invalid_binding")?;
    if vault::exists(&binding).map_err(|_| "credential_lookup_failed")? {
        return Err("unexpected_existing_binding");
    }
    let cleanup = Cleanup(binding);
    let outcome = (|| {
        vault::store(&cleanup.0, &secret).map_err(|_| "credential_store_failed")?;
        let worker = fixture
            .app
            .inference_jobs_worker()
            .map_err(|_| "worker_setup_failed")?
            .ok_or("worker_setup_failed")?;
        let config = config(root.path());
        let shutdown = config.shutdown.clone();
        let mut mismatched = false;
        worker
            .run_with_progress(
                config,
                &mut |identity, answer, _| {
                    if identity.local_instance_id != fixture.identity.local_instance_id
                        || identity.workspace_id != fixture.identity.workspace_id
                    {
                        shutdown.store(true, Ordering::Release);
                    }
                    if answer.is_some_and(|answer| {
                        answer.id == fixture.answer_id
                            && answer.status != AnswerStatus::Queued
                            && answer.status != AnswerStatus::Running
                    }) {
                        // The one accepted answer is complete; do not start unrelated indexing.
                        shutdown.store(true, Ordering::Release);
                    }
                },
                &mut |identity, progress| {
                    if identity.local_instance_id != fixture.identity.local_instance_id
                        || identity.workspace_id != fixture.identity.workspace_id
                        || progress.answer_id != fixture.answer_id
                    {
                        mismatched = true;
                        shutdown.store(true, Ordering::Release);
                    } else {
                        record_progress(report, progress);
                    }
                },
            )
            .map_err(|_| "agent_worker_failed")?;
        if mismatched {
            return Err("agent_progress_identity_mismatch");
        }
        let answer = fixture
            .app
            .library(&fixture.identity)
            .map_err(|_| "answer_read_failed")?
            .answer(&fixture.answer_id)
            .map_err(|_| "answer_read_failed")?;
        verify_answer(&answer, &fixture.note_ids, report)
    })();
    cleanup.checked()?;
    report.credential_removed = true;
    outcome
}

fn main() {
    let mut report = Report::default();
    let result = read_input(std::io::stdin().lock()).and_then(|input| probe(input, &mut report));
    report.code = match result {
        Ok(()) => "verified_synthetic_agent_answer",
        Err(code) => code,
    };
    // Only locally defined static codes/counters; never provider bodies, text or profile.
    println!(
        "{}",
        serde_json::to_string(&report).expect("static report serialization")
    );
    if result.is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> GenerationProfile {
        GenerationProfile {
            channel_id: None,
            capabilities: None,
            provider: "openai_chat".into(),
            endpoint: "https://example.invalid/v1".into(),
            model: "offline-test".into(),
            model_digest: String::new(),
            config_revision: 1,
            temperature: 0.0,
            context_tokens: 32768,
            chat_token_limit: None,
        }
    }
    #[test]
    fn bounded_single_line_input_does_not_wait_for_or_consume_a_second_record() {
        let json = serde_json::json!({"profile":profile(),"api_key":"synthetic-key"});
        let text = format!("{json}\nnot a second JSON line");
        let mut reader = std::io::Cursor::new(text.as_bytes());
        let input = read_input(&mut reader).unwrap();
        assert_eq!(input.profile.model, "offline-test");
        assert_eq!(reader.position() as usize, json.to_string().len() + 1);
        assert!(matches!(
            read_input(std::io::Cursor::new(vec![b'x'; INPUT_LIMIT + 1])),
            Err("input_too_large")
        ));
    }
    #[test]
    fn real_app_setup_is_lexical_scoped_and_unexecuted_without_credentials() {
        let root = tempfile::tempdir().unwrap();
        let fixture = setup(root.path(), profile()).unwrap();
        let library = fixture.app.library(&fixture.identity).unwrap();
        let answer = library.answer(&fixture.answer_id).unwrap();
        assert_eq!(answer.status, AnswerStatus::Queued);
        assert_eq!(answer.request.search.mode, SearchMode::Lexical);
        assert_eq!(answer.request.search.scope.object_ids, fixture.note_ids);
        assert_eq!(answer.request.agent.unwrap().max_model_calls, 4);
        assert!(library.active_embedding_profile().unwrap().is_none());
        let mut query = answer.request.search;
        query.query = "松柏计划".into();
        let found = library.keyword_search(&query).unwrap();
        assert_eq!(found.items.len(), 2);
        assert!(
            found
                .items
                .iter()
                .all(|item| fixture.note_ids.contains(&item.target.object_id))
        );
    }
    #[test]
    fn completed_without_tool_activity_cannot_be_reported_as_agent_success() {
        let root = tempfile::tempdir().unwrap();
        let fixture = setup(root.path(), profile()).unwrap();
        let mut answer = fixture
            .app
            .library(&fixture.identity)
            .unwrap()
            .answer(&fixture.answer_id)
            .unwrap();
        answer.status = AnswerStatus::Completed;
        assert_eq!(
            verify_answer(&answer, &fixture.note_ids, &Report::default()),
            Err("agent_tool_loop_not_verified")
        );
    }

    #[test]
    fn fact_checks_require_the_right_label_value_and_source_association() {
        let root = tempfile::tempdir().unwrap();
        let fixture = setup(root.path(), profile()).unwrap();
        let library = fixture.app.library(&fixture.identity).unwrap();
        let mut answer = library.answer(&fixture.answer_id).unwrap();
        let mut query = answer.request.search.clone();
        query.query = "松柏计划".into();
        let found = library.keyword_search(&query).unwrap().items;
        answer.evidence = fixture
            .note_ids
            .iter()
            .enumerate()
            .map(|(index, id)| pkms_core::AnswerEvidence {
                citation_id: format!("E{}", index + 1),
                context: None,
                evidence: found
                    .iter()
                    .find(|item| &item.target.object_id == id)
                    .unwrap()
                    .clone(),
            })
            .collect();
        answer.cited_ids = vec!["E1".into(), "E2".into()];
        answer.evidence_current = true;
        answer.status = AnswerStatus::Completed;
        let report = Report {
            model_calls: 3,
            tool_calls: 2,
            search_stages: 1,
            ..Default::default()
        };
        answer.text = "- 晨间保留7天。 [E1]\n- 晚间保留13天。 [E2]".into();
        assert!(verify_answer(&answer, &fixture.note_ids, &report).is_ok());
        for text in [
            "- 晨间保留13天。 [E1]\n- 晚间保留7天。 [E2]",
            "- 晨间保留7天。 [E2]\n- 晚间保留13天。 [E1]",
            "- 晨间保留13天，晚间保留7天。 [E1][E2]",
        ] {
            answer.text = text.into();
            assert_eq!(
                verify_answer(&answer, &fixture.note_ids, &report),
                Err("synthetic_facts_not_verified")
            );
        }
    }
}

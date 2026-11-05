//! One bounded external research request. The host performs exactly one GET,
//! reusing the same URL validation, size limits and cancellation checkpoints as
//! public webpage capture. Only the declared query leaves the machine.
use crate::{AppError, AppService, Result, WorkspaceIdentity, webpage_capture};
use pkms_core::{
    Library, RecordExternalStep, RecordFailedExternalStep, ResearchCommit, ResearchStatus,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct ExternalResearchPlan {
    pub task_id: String,
    pub base_row_version: u32,
    pub query: String,
    pub endpoint: String,
    pub url: String,
}

#[derive(Debug)]
pub enum ExternalResearchPreparation {
    Replayed(Box<ResearchCommit>),
    Plan(ExternalResearchPlan),
}

#[derive(Debug)]
pub struct ExternalResearchOutcome {
    pub http_status: u16,
    pub final_url: String,
    pub title: String,
    pub excerpt: String,
}

fn encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn error_code(error: &AppError) -> String {
    match error {
        AppError::InvalidInput(code) => (*code).to_owned(),
        AppError::Core(pkms_core::Error::InvalidInput(code)) => (*code).to_owned(),
        _ => "research_external_failed".to_owned(),
    }
}

impl AppService {
    /// Checks committed host requests before looking at the current task or URL.
    /// Both the synchronous entry and Tauri's unlocked fetch path use this gate.
    pub fn prepare_external_research(
        &self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        task_id: &str,
        base_row_version: u32,
    ) -> Result<ExternalResearchPreparation> {
        if let Some(result) = self.library(identity)?.replay_external_research(
            command_id,
            task_id,
            base_row_version,
        )? {
            return Ok(ExternalResearchPreparation::Replayed(Box::new(result)));
        }
        self.research_external_plan(identity, task_id, base_row_version)
            .map(ExternalResearchPreparation::Plan)
    }

    pub fn research_external_plan(
        &self,
        identity: &WorkspaceIdentity,
        task_id: &str,
        base_row_version: u32,
    ) -> Result<ExternalResearchPlan> {
        let task = self.library(identity)?.research_task(task_id)?;
        if task.row_version != base_row_version {
            return Err(AppError::InvalidInput("research_task_changed"));
        }
        if task.status == ResearchStatus::Canceled {
            return Err(AppError::InvalidInput("research_canceled"));
        }
        if !matches!(
            task.status,
            ResearchStatus::Pending | ResearchStatus::Running
        ) {
            return Err(AppError::InvalidInput("research_not_running"));
        }
        if !task.external_allowed {
            return Err(AppError::InvalidInput("research_external_not_allowed"));
        }
        let endpoint = task
            .external_search_endpoint
            .clone()
            .ok_or(AppError::InvalidInput("research_external_not_configured"))?;
        let query = task
            .queries
            .get(task.step_count as usize)
            .cloned()
            .ok_or(AppError::InvalidInput("research_queries_exhausted"))?;
        let url = endpoint.replace("{query}", &encode_query(&query));
        webpage_capture::validate_url(&url)?;
        Ok(ExternalResearchPlan {
            task_id: task.id,
            base_row_version,
            query,
            endpoint,
            url,
        })
    }

    pub fn run_external_research(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        task_id: &str,
        base_row_version: u32,
    ) -> Result<ResearchCommit> {
        let plan = match self.prepare_external_research(
            identity,
            command_id,
            task_id,
            base_row_version,
        )? {
            ExternalResearchPreparation::Replayed(result) => return Ok(*result),
            ExternalResearchPreparation::Plan(plan) => plan,
        };
        let path = self.workspace_path(identity)?;
        let checker = Library::open_existing(&path)?;
        let shutdown = AtomicBool::new(false);
        let outcome = fetch_external_research(&plan, &checker, &shutdown);
        self.record_external_research(identity, command_id, plan, outcome)
    }

    pub fn record_external_research(
        &mut self,
        identity: &WorkspaceIdentity,
        command_id: &str,
        plan: ExternalResearchPlan,
        outcome: std::result::Result<ExternalResearchOutcome, String>,
    ) -> Result<ResearchCommit> {
        match outcome {
            Ok(outcome) => self
                .library_mut(identity)?
                .record_external_step(
                    command_id,
                    RecordExternalStep {
                        task_id: plan.task_id,
                        base_row_version: plan.base_row_version,
                        query: plan.query,
                        endpoint: plan.endpoint,
                        http_status: outcome.http_status,
                        final_url: outcome.final_url,
                        title: outcome.title,
                        excerpt: outcome.excerpt,
                    },
                )
                .map_err(Into::into),
            Err(code) => self
                .library_mut(identity)?
                .record_failed_external_step(
                    command_id,
                    RecordFailedExternalStep {
                        task_id: plan.task_id,
                        base_row_version: plan.base_row_version,
                        query: plan.query,
                        endpoint: plan.endpoint,
                        error_code: code,
                    },
                )
                .map_err(Into::into),
        }
    }
}

/// Blocking fetch with a cancellation check every 100 ms. The checker reads a
/// separate connection so a cancel committed while this request is in flight is
/// observed before the next chunk or redirect.
pub fn fetch_external_research(
    plan: &ExternalResearchPlan,
    checker: &Library,
    shutdown: &AtomicBool,
) -> std::result::Result<ExternalResearchOutcome, String> {
    let mut check = || {
        if shutdown.load(Ordering::Acquire) {
            return Err(AppError::InvalidInput("research_canceled"));
        }
        match checker.research_task(&plan.task_id) {
            Ok(task) if task.status == ResearchStatus::Canceled => {
                Err(AppError::InvalidInput("research_canceled"))
            }
            Ok(_) => Ok(()),
            Err(_) => Err(AppError::InvalidInput("research_canceled")),
        }
    };
    let capture =
        webpage_capture::capture(&plan.url, &mut check).map_err(|error| error_code(&error))?;
    let html = std::fs::read_to_string(capture.file.path())
        .map_err(|_| "research_external_unreadable".to_owned())?;
    let text = html_to_text(&html);
    if text.trim().is_empty() {
        return Err("research_external_empty".to_owned());
    }
    let title = extract_title(&html);
    Ok(ExternalResearchOutcome {
        http_status: capture.capture.http_status,
        final_url: capture.capture.final_url,
        title: if title.is_empty() {
            plan.query.clone()
        } else {
            title
        },
        excerpt: text.chars().take(1200).collect(),
    })
}

fn extract_title(html: &str) -> String {
    let lower = html.to_lowercase();
    let Some(start) = lower.find("<title") else {
        return String::new();
    };
    let Some(open) = lower[start..].find('>') else {
        return String::new();
    };
    let body_start = start + open + 1;
    let Some(end) = lower[body_start..].find("</title>") else {
        return String::new();
    };
    decode_entities(html[body_start..body_start + end].trim())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut chars = html.chars().peekable();
    let mut skipping: Option<String> = None;
    while let Some(character) = chars.next() {
        if character == '<' {
            let mut tag = String::new();
            for next in chars.by_ref() {
                if next == '>' {
                    break;
                }
                tag.push(next);
            }
            let closing = tag.trim_start().starts_with('/');
            let name = tag
                .trim()
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if !closing && matches!(name.as_str(), "script" | "style") {
                skipping = Some(name);
            } else if closing && skipping.as_deref() == Some(name.as_str()) {
                skipping = None;
            }
            out.push(' ');
            continue;
        }
        if skipping.is_none() {
            out.push(character);
        }
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

use clap::{Parser, Subcommand};
use pkms_core::{
    ExternalScope, Library, ReportDraft, ReportTemplate, SearchMode, SearchQuery, SearchScope,
};
use pkms_external::{
    ExternalService, McpSession, ServicePolicy, TokenStore,
    report_export::{PreparedReport, ReportFormat, export_reports},
    serve_stdio,
};
use pkms_inference::remote::{ApiKey, Client, Finish, Limits, Message, Protocol, Request};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(about = "spanseek external MCP/HTTP adapter", version)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Serve the read-only MCP stdio server for one workspace.
    ServeMcp {
        #[arg(long)]
        workspace: PathBuf,
        /// Expose a proposal-only tool. It still never applies the proposal.
        #[arg(long)]
        enable_proposals: bool,
        /// Expose the audit reader.
        #[arg(long)]
        enable_audit: bool,
    },
    /// Serve the loopback HTTP API with bearer-token authentication.
    ServeHttp {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long, default_value_t = 8787)]
        port: u16,
        #[arg(long)]
        enable_proposals: bool,
        #[arg(long)]
        enable_audit: bool,
    },
    /// Create a bearer token. The plaintext is printed once and never stored.
    TokenCreate {
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        label: String,
        #[arg(long, value_delimiter = ',', default_value = "read,search")]
        scopes: Vec<String>,
    },
    /// List token metadata without any secret material.
    TokenList {
        #[arg(long)]
        token_file: PathBuf,
    },
    /// Revoke a token by id.
    TokenRevoke {
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        id: String,
    },
    /// Read one pending or historical proposal for human review.
    ProposalGet {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        id: String,
    },
    /// Apply a reviewed proposal to the note. Requires explicit confirmation.
    ProposalApply {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Reject a reviewed proposal without touching the note.
    ProposalReject {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        id: String,
        #[arg(long)]
        confirm: bool,
    },
    /// Export one or more cited report manifests as Markdown, DOCX and PDF.
    ExportReports {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long = "input", required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long, value_delimiter = ',', default_value = "md,docx,pdf")]
        formats: Vec<String>,
        #[arg(long)]
        output_dir: PathBuf,
    },
    /// Re-resolve every citation in a report manifest against the workspace.
    VerifyReport {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Build a cited report draft from search evidence.
    DraftReport {
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        query: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        question: String,
        /// evidence or model
        #[arg(long, default_value = "evidence")]
        mode: String,
        #[arg(long, default_value_t = 10)]
        limit: u32,
        #[arg(long)]
        template: Option<PathBuf>,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        markdown: Option<PathBuf>,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// chat-completions, responses, anthropic-messages or gemini
        #[arg(long, default_value = "chat-completions")]
        protocol: String,
        #[arg(long, default_value = "SPANSEEK_MODEL_API_KEY")]
        api_key_env: String,
        #[arg(long, default_value_t = 4096)]
        max_output_tokens: u32,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    match Arguments::parse().command {
        Command::ServeMcp {
            workspace,
            enable_proposals,
            enable_audit,
        } => {
            let library = Library::open_existing(&workspace)?;
            let service = ExternalService::new(
                library,
                ServicePolicy {
                    allow_proposals: enable_proposals,
                    allow_audit: enable_audit,
                },
            );
            serve_stdio(McpSession::new(service))?;
        }
        Command::ServeHttp {
            workspace,
            token_file,
            port,
            enable_proposals,
            enable_audit,
        } => {
            let library = Library::open_existing(&workspace)?;
            let service = ExternalService::new(
                library,
                ServicePolicy {
                    allow_proposals: enable_proposals,
                    allow_audit: enable_audit,
                },
            );
            let tokens = TokenStore::open(&token_file)?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(pkms_external::serve_http(port, service, tokens))?;
        }
        Command::TokenCreate {
            token_file,
            label,
            scopes,
        } => {
            let scopes = scopes
                .iter()
                .map(|scope| parse_scope(scope))
                .collect::<Result<Vec<_>, _>>()?;
            let mut store = TokenStore::open(&token_file)?;
            let created = store.create(&label, scopes)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "token": created.token,
                    "token_id": created.summary.id,
                    "label": created.summary.label,
                    "scopes": created.summary.scopes,
                    "created_at": created.summary.created_at,
                    "notice": "This plaintext token is shown once and is not stored."
                }))?
            );
        }
        Command::TokenList { token_file } => {
            let store = TokenStore::open(&token_file)?;
            println!("{}", serde_json::to_string_pretty(&store.list())?);
        }
        Command::TokenRevoke { token_file, id } => {
            let mut store = TokenStore::open(&token_file)?;
            println!("{}", serde_json::to_string_pretty(&store.revoke(&id)?)?);
        }
        Command::ProposalGet { workspace, id } => {
            let library = Library::open_existing(&workspace)?;
            println!("{}", serde_json::to_string_pretty(&library.proposal(&id)?)?);
        }
        Command::ProposalApply {
            workspace,
            id,
            confirm,
        } => {
            if !confirm {
                return Err(cli_error("confirmation_required"));
            }
            let mut library = Library::open_existing(&workspace)?;
            let commit = library.apply_proposal(&uuid::Uuid::new_v4().to_string(), &id)?;
            println!("{}", serde_json::to_string_pretty(&commit)?);
        }
        Command::ProposalReject {
            workspace,
            id,
            confirm,
        } => {
            if !confirm {
                return Err(cli_error("confirmation_required"));
            }
            let mut library = Library::open_existing(&workspace)?;
            let mutation = library.reject_proposal(&uuid::Uuid::new_v4().to_string(), &id)?;
            println!("{}", serde_json::to_string_pretty(&mutation)?);
        }
        Command::ExportReports {
            workspace,
            inputs,
            formats,
            output_dir,
        } => {
            let library = Library::open_existing(&workspace)?;
            let formats = formats
                .iter()
                .map(|format| ReportFormat::parse(format))
                .collect::<Result<Vec<_>, _>>()?;
            let mut reports = Vec::new();
            for input in inputs {
                let bytes = fs::read(&input)?;
                let report: ReportDraft = serde_json::from_slice(&bytes)?;
                reports.push(PreparedReport {
                    source: input,
                    report,
                });
            }
            let manifest = export_reports(&library, reports, &formats, &output_dir)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Command::VerifyReport { workspace, report } => {
            let library = Library::open_existing(&workspace)?;
            let bytes = fs::read(&report)?;
            let report: ReportDraft = serde_json::from_slice(&bytes)?;
            let verification = library.verify_report(&report)?;
            println!("{}", serde_json::to_string_pretty(&verification)?);
            if !verification.all_valid {
                return Err(cli_error("report_citation_verification_failed"));
            }
        }
        Command::DraftReport {
            workspace,
            query,
            title,
            question,
            mode,
            limit,
            template,
            output,
            markdown,
            endpoint,
            model,
            protocol,
            api_key_env,
            max_output_tokens,
        } => {
            let library = Library::open_existing(&workspace)?;
            let results = library.keyword_search(&SearchQuery {
                query,
                mode: SearchMode::Lexical,
                scope: SearchScope::default(),
                limit,
                minimum_cosine: None,
            })?;
            if results.items.is_empty() {
                return Err(cli_error("no_evidence"));
            }
            let template = match template {
                Some(path) => ReportTemplate::load(path)?,
                None => ReportTemplate::built_in()?,
            };
            let report = match mode.as_str() {
                "evidence" => {
                    library.build_evidence_report(&title, &question, results.items, &template)?
                }
                "model" => {
                    let (endpoint, model) = match (endpoint, model) {
                        (Some(endpoint), Some(model))
                            if !endpoint.trim().is_empty() && !model.trim().is_empty() =>
                        {
                            (endpoint, model)
                        }
                        _ => return Err(cli_error("model_unconfigured")),
                    };
                    let api_key = std::env::var(&api_key_env)
                        .ok()
                        .filter(|value| !value.is_empty())
                        .ok_or_else(|| cli_error("model_unconfigured"))?;
                    let body = generate_model_body(
                        &endpoint,
                        &model,
                        &protocol,
                        &api_key,
                        &title,
                        &question,
                        results.items.clone(),
                        max_output_tokens,
                    )?;
                    library.build_model_report(
                        &title,
                        &question,
                        &body,
                        results.items,
                        &template,
                    )?
                }
                _ => return Err(cli_error("invalid_report_mode")),
            };
            let verification = library.verify_report(&report)?;
            if !verification.all_valid {
                return Err(cli_error("report_citation_verification_failed"));
            }
            let markdown_path = markdown.unwrap_or_else(|| output.with_extension("md"));
            let verification_path = output.with_file_name(format!(
                "{}.verification.json",
                output
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("report")
            ));
            let report_bytes = format!("{}\n", serde_json::to_string_pretty(&report)?);
            let verification_bytes = format!("{}\n", serde_json::to_string_pretty(&verification)?);
            write_new(&output, report_bytes.as_bytes())?;
            if let Err(error) = write_new(&markdown_path, report.markdown.as_bytes()) {
                let _ = fs::remove_file(&output);
                return Err(error);
            }
            if let Err(error) = write_new(&verification_path, verification_bytes.as_bytes()) {
                let _ = fs::remove_file(&output);
                let _ = fs::remove_file(&markdown_path);
                return Err(error);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "status": "report_drafted",
                    "mode": mode,
                    "generated_by": report.generated_by,
                    "citations": report.citations.len(),
                    "all_valid": verification.all_valid,
                    "report": output,
                    "markdown": markdown_path,
                    "verification": verification_path,
                }))?
            );
        }
    }
    Ok(())
}

fn parse_scope(value: &str) -> Result<ExternalScope, Box<dyn std::error::Error>> {
    Ok(match value {
        "read" => ExternalScope::Read,
        "search" => ExternalScope::Search,
        "propose" => ExternalScope::Propose,
        "audit" => ExternalScope::Audit,
        _ => return Err(format!("unknown scope: {value}").into()),
    })
}

fn parse_protocol(value: &str) -> Result<Protocol, Box<dyn std::error::Error>> {
    Ok(match value {
        "chat-completions" => Protocol::ChatCompletions,
        "responses" => Protocol::Responses,
        "anthropic-messages" => Protocol::AnthropicMessages,
        "gemini" => Protocol::Gemini,
        _ => return Err(cli_error("unsupported_model_protocol")),
    })
}

#[allow(clippy::too_many_arguments)]
fn generate_model_body(
    endpoint: &str,
    model: &str,
    protocol: &str,
    api_key: &str,
    title: &str,
    question: &str,
    evidence: Vec<pkms_core::SearchEvidence>,
    max_output_tokens: u32,
) -> Result<String, Box<dyn std::error::Error>> {
    let protocol = parse_protocol(protocol)?;
    let key = ApiKey::new(api_key.to_owned()).map_err(|_| cli_error("model_invalid_key"))?;
    let evidence = evidence
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "citation": format!("E{}", index + 1),
                "title": item.title,
                "kind": item.target.kind,
                "object_id": item.target.object_id,
                "content_revision": item.target.content_revision,
                "block_id": item.target.block_id,
                "text": item.text,
            })
        })
        .collect::<Vec<_>>();
    let system = "你是 spanseek 的有据报告起草器。只能使用用户提供的证据。每个非标题段落都必须包含至少一个 [E编号] 引用；不得补写证据中没有的事实；无法支持时保持简短并明确说明证据不足。不要输出来源列表，程序会追加。";
    let user = format!(
        "报告标题：{title}\n研究问题：{question}\n\n冻结证据：\n{}\n\n请输出 Markdown 正文，不要重复报告标题。",
        serde_json::to_string_pretty(&evidence)?
    );
    let request = Request {
        model: model.into(),
        messages: vec![
            Message::System {
                text: system.into(),
            },
            Message::User { text: user },
        ],
        tools: Vec::new(),
        max_output_tokens,
        chat_token_limit: None,
        temperature: Some(0.2),
        output_format: None,
    };
    let client = Client::new(protocol, endpoint, Limits::default())
        .map_err(|error| cli_error(error.code()))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let completion = runtime
        .block_on(client.generate(&request, &key, |_| Ok(()), || false))
        .map_err(|error| cli_error(error.code()))?;
    if completion.finish != Finish::Stop || completion.text.trim().is_empty() {
        return Err(cli_error("model_incomplete"));
    }
    Ok(completion.text)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn cli_error(message: &'static str) -> Box<dyn std::error::Error> {
    std::io::Error::other(message).into()
}

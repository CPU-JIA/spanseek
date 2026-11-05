//! Cited report drafts built from frozen search evidence.
//!
//! The extractive path never invents prose: every paragraph is an evidence
//! excerpt with a stable citation ID. The model path accepts text only after
//! every non-heading paragraph cites a listed evidence item. Neither path writes
//! knowledge; the caller decides whether to save a reviewable proposal.
use crate::{
    Error, Library, ReportCitation, ReportDraft, ReportSource, ReportTemplate, ReportVerification,
    Result, SearchEvidence, SearchScope,
};
use std::{collections::BTreeSet, fs, path::Path};

const DEFAULT_TEMPLATE: &str = include_str!("../templates/report-default.json");
const MAX_REPORT_EVIDENCE: usize = 32;
const MAX_REPORT_BODY_BYTES: usize = 512 * 1024;

impl ReportTemplate {
    pub fn built_in() -> Result<Self> {
        Self::parse(DEFAULT_TEMPLATE)
    }

    pub fn parse(json: &str) -> Result<Self> {
        let template: Self = serde_json::from_str(json)?;
        template.validate()?;
        Ok(template)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = fs::read(path)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::InvalidInput("report_template_must_be_utf8"))?;
        Self::parse(text)
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.name.trim().is_empty()
            || self.name.chars().count() > 120
        {
            return Err(Error::InvalidInput("invalid_report_template_header"));
        }
        validate_placeholders(&self.title, &["title"], &["title"])?;
        validate_placeholders(&self.question, &["question"], &["question"])?;
        validate_placeholders(&self.section_heading, &["heading"], &["heading"])?;
        validate_placeholders(
            &self.paragraph,
            &["text", "citation"],
            &["text", "citation"],
        )?;
        validate_placeholders(&self.sources_heading, &[], &[])?;
        validate_placeholders(
            &self.source_line,
            &["citation", "title", "locator"],
            &["citation", "title", "locator"],
        )?;
        validate_placeholders(&self.citation_format, &["number"], &["number"])?;
        Ok(())
    }
}

impl Library {
    pub fn build_evidence_report(
        &self,
        title: &str,
        question: &str,
        evidence: Vec<SearchEvidence>,
        template: &ReportTemplate,
    ) -> Result<ReportDraft> {
        validate_request(title, question, &evidence)?;
        let citations = citations(evidence);
        for citation in &citations {
            if !self.validate_search_evidence(&citation.evidence, &SearchScope::default())? {
                return Err(Error::InvalidInput("report_evidence_is_stale_or_forged"));
            }
        }
        let sources = sources(&citations);
        let markdown = render_extractive(title, question, &citations, &sources, template)?;
        Ok(ReportDraft {
            schema_version: 1,
            title: title.into(),
            question: question.into(),
            generated_by: "evidence_extractive".into(),
            template_name: template.name.clone(),
            markdown,
            citations,
            sources,
            warnings: vec!["摘录式报告逐条保留检索证据，不对未出现在证据中的事实作补全。".into()],
        })
    }

    pub fn build_model_report(
        &self,
        title: &str,
        question: &str,
        body: &str,
        evidence: Vec<SearchEvidence>,
        template: &ReportTemplate,
    ) -> Result<ReportDraft> {
        validate_request(title, question, &evidence)?;
        if body.trim().is_empty() || body.len() > MAX_REPORT_BODY_BYTES {
            return Err(Error::InvalidInput("invalid_model_report_body"));
        }
        let citations = citations(evidence);
        let ids = citations
            .iter()
            .map(|citation| citation.id.as_str())
            .collect::<BTreeSet<_>>();
        validate_cited_body(body, &ids)?;
        for citation in &citations {
            if !self.validate_search_evidence(&citation.evidence, &SearchScope::default())? {
                return Err(Error::InvalidInput("report_evidence_is_stale_or_forged"));
            }
        }
        let sources = sources(&citations);
        let mut markdown = format!(
            "{}\n\n{}\n\n{}\n\n{}\n",
            fill(&template.title, &[("title", title)]),
            fill(&template.question, &[("question", question)]),
            body.trim(),
            render_sources(&sources, template)?
        );
        if markdown.ends_with("\\n\\n") {
            markdown.pop();
        }
        Ok(ReportDraft {
            schema_version: 1,
            title: title.into(),
            question: question.into(),
            generated_by: "model_cited".into(),
            template_name: template.name.clone(),
            markdown,
            citations,
            sources,
            warnings: vec![
                "正文由已配置模型基于列出的 E 编号证据生成；每段必须含有效引用。".into(),
            ],
        })
    }

    pub fn verify_report(&self, report: &ReportDraft) -> Result<ReportVerification> {
        if report.schema_version != 1 || report.citations.is_empty() {
            return Err(Error::InvalidInput("invalid_report_manifest"));
        }
        let known = report
            .citations
            .iter()
            .map(|citation| citation.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut invalid = BTreeSet::new();
        let markers = citation_markers(&report.markdown)?;
        for marker in &markers {
            if !known.contains(marker.as_str()) {
                invalid.insert(marker.clone());
            }
        }
        for citation in &report.citations {
            if !markers.contains(&citation.id) {
                invalid.insert(citation.id.clone());
            }
        }
        if markers.is_empty() {
            return Err(Error::InvalidInput("report_has_no_citations"));
        }
        for citation in &report.citations {
            if invalid.contains(&citation.id) {
                continue;
            }
            if !self.validate_search_evidence(&citation.evidence, &SearchScope::default())? {
                invalid.insert(citation.id.clone());
            }
        }
        let total = report.citations.len() as u32;
        let invalid_citation_ids = invalid.into_iter().collect::<Vec<_>>();
        Ok(ReportVerification {
            total,
            valid: total.saturating_sub(invalid_citation_ids.len() as u32),
            all_valid: invalid_citation_ids.is_empty(),
            invalid_citation_ids,
        })
    }
}

fn validate_request(title: &str, question: &str, evidence: &[SearchEvidence]) -> Result<()> {
    if title.trim().is_empty() || title.chars().count() > 300 || question.chars().count() > 4000 {
        return Err(Error::InvalidInput("invalid_report_title_or_question"));
    }
    if evidence.is_empty() {
        return Err(Error::InvalidInput("no_evidence"));
    }
    if evidence.len() > MAX_REPORT_EVIDENCE {
        return Err(Error::InvalidInput("too_many_report_evidence_items"));
    }
    let mut ids = BTreeSet::new();
    for item in evidence {
        if item.title.trim().is_empty()
            || item.text.trim().is_empty()
            || item.text.len() > 64 * 1024
            || !ids.insert(item.id.as_str())
        {
            return Err(Error::InvalidInput("invalid_report_evidence"));
        }
    }
    Ok(())
}

fn citations(evidence: Vec<SearchEvidence>) -> Vec<ReportCitation> {
    evidence
        .into_iter()
        .enumerate()
        .map(|(index, evidence)| ReportCitation {
            id: format!("E{}", index + 1),
            evidence,
        })
        .collect()
}

fn sources(citations: &[ReportCitation]) -> Vec<ReportSource> {
    citations
        .iter()
        .map(|citation| {
            let evidence = &citation.evidence;
            let mut locator = format!(
                "revision {} / block {}",
                evidence.target.content_revision, evidence.target.block_id
            );
            if let Some(page) = evidence.target.page_index {
                locator.push_str(&format!(" / page {}", page + 1));
            }
            ReportSource {
                citation_id: citation.id.clone(),
                object_id: evidence.target.object_id.clone(),
                kind: evidence.target.kind,
                title: evidence.title.clone(),
                content_revision: evidence.target.content_revision.clone(),
                locator,
            }
        })
        .collect()
}

fn render_extractive(
    title: &str,
    question: &str,
    citations: &[ReportCitation],
    sources: &[ReportSource],
    template: &ReportTemplate,
) -> Result<String> {
    let mut lines = vec![fill(&template.title, &[("title", title)])];
    if !question.trim().is_empty() {
        lines.push(String::new());
        lines.push(fill(&template.question, &[("question", question.trim())]));
    }
    let mut groups: Vec<(String, Vec<&ReportCitation>)> = Vec::new();
    for citation in citations {
        if let Some((_, values)) = groups
            .iter_mut()
            .find(|(object_id, _)| object_id == &citation.evidence.target.object_id)
        {
            values.push(citation);
        } else {
            groups.push((citation.evidence.target.object_id.clone(), vec![citation]));
        }
    }
    for (_, group) in groups {
        lines.push(String::new());
        lines.push(fill(
            &template.section_heading,
            &[("heading", &group[0].evidence.title)],
        ));
        for citation in group {
            lines.push(String::new());
            let marker = fill(&template.citation_format, &[("number", &citation.id[1..])]);
            lines.push(fill(
                &template.paragraph,
                &[
                    ("text", citation.evidence.text.trim()),
                    ("citation", &marker),
                ],
            ));
        }
    }
    lines.push(String::new());
    lines.push(render_sources(sources, template)?);
    Ok(format!("{}\n", lines.join("\n")))
}

fn render_sources(sources: &[ReportSource], template: &ReportTemplate) -> Result<String> {
    let mut lines = vec![template.sources_heading.clone()];
    for source in sources {
        lines.push(fill(
            &template.source_line,
            &[
                ("citation", &source.citation_id),
                ("title", &source.title),
                ("locator", &source.locator),
            ],
        ));
    }
    Ok(lines.join("\n"))
}

fn validate_cited_body(body: &str, known: &BTreeSet<&str>) -> Result<()> {
    if body.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && trimmed != "---"
            && !trimmed.contains("[E")
    }) {
        return Err(Error::InvalidInput("report_paragraph_without_citation"));
    }
    let markers = citation_markers(body)?;
    if markers.is_empty()
        || markers
            .iter()
            .any(|marker| !known.contains(marker.as_str()))
    {
        return Err(Error::InvalidInput("report_has_invalid_citation"));
    }
    Ok(())
}

fn citation_markers(text: &str) -> Result<Vec<String>> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let mut markers = Vec::new();
    while index + 1 < bytes.len() {
        let opens = bytes[index] == b'[' || bytes[index] == b'(';
        if opens && bytes[index + 1] == b'E' {
            let mut end = index + 2;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end == index + 2 {
                return Err(Error::InvalidInput("report_has_malformed_citation"));
            }
            if end >= bytes.len() {
                return Err(Error::InvalidInput("report_has_malformed_citation"));
            }
            let expected_close = if bytes[index] == b'[' { b']' } else { b')' };
            if bytes[end] != expected_close {
                return Err(Error::InvalidInput("report_has_malformed_citation"));
            }
            markers.push(text[index + 1..end].to_owned());
            index = end + 1;
        } else {
            index += 1;
        }
    }
    Ok(markers)
}

fn validate_placeholders(text: &str, allowed: &[&str], required: &[&str]) -> Result<()> {
    let found = placeholders(text)?;
    if found.iter().any(|name| !allowed.contains(&name.as_str()))
        || required
            .iter()
            .any(|name| !found.iter().any(|value| value == name))
    {
        return Err(Error::InvalidInput("unknown_report_template_placeholder"));
    }
    Ok(())
}

fn placeholders(text: &str) -> Result<Vec<String>> {
    let mut rest = text;
    let mut found = Vec::new();
    while let Some(open) = rest.find("{{") {
        if rest[..open].contains("}}") {
            return Err(Error::InvalidInput("unexpected_report_template_brace"));
        }
        let tail = &rest[open + 2..];
        let close = tail
            .find("}}")
            .ok_or(Error::InvalidInput("unclosed_report_template_placeholder"))?;
        let name = &tail[..close];
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err(Error::InvalidInput("invalid_report_template_placeholder"));
        }
        found.push(name.to_owned());
        rest = &tail[close + 2..];
    }
    if rest.contains("}}") {
        return Err(Error::InvalidInput("unexpected_report_template_brace"));
    }
    Ok(found)
}

fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = template.to_owned();
    for (key, value) in values {
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_templates_are_explicit_and_parseable() {
        let default = ReportTemplate::built_in().unwrap();
        assert_eq!(default.name, "evidence-report-default");
        assert!(default.paragraph.contains("{{citation}}"));
        let brief = ReportTemplate::parse(include_str!("../templates/report-brief.json")).unwrap();
        assert_eq!(brief.name, "evidence-report-brief");
    }

    #[test]
    fn unknown_placeholders_and_malformed_citations_are_rejected() {
        let mut value: serde_json::Value = serde_json::from_str(DEFAULT_TEMPLATE).unwrap();
        value["paragraph"] = serde_json::json!("{{text}} {{unknown}}");
        assert!(matches!(
            ReportTemplate::parse(&value.to_string()),
            Err(Error::InvalidInput("unknown_report_template_placeholder"))
        ));
        assert!(matches!(
            citation_markers("claim [E99] and [E]"),
            Err(Error::InvalidInput("report_has_malformed_citation"))
        ));
    }

    #[test]
    fn model_body_requires_a_valid_citation_in_every_paragraph() {
        let known = BTreeSet::from(["E1"]);
        assert!(validate_cited_body("## Heading\n\nA cited claim [E1].", &known).is_ok());
        assert!(matches!(
            validate_cited_body("An uncited claim.", &known),
            Err(Error::InvalidInput("report_paragraph_without_citation"))
        ));
        assert!(matches!(
            validate_cited_body("A forged claim [E2].", &known),
            Err(Error::InvalidInput("report_has_invalid_citation"))
        ));
    }
}

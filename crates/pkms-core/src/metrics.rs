//! Local operational metrics: retrieval latency and model call/token usage.
//!
//! Rows carry timestamps, coarse classification and numbers only — never user
//! text, prompts or file paths. They are pruned to a rolling window so the
//! workspace file stays bounded, and they are restored with the rest of the file.
use crate::{Error, Library, ModelUsageStats, Result, RetrievalLatencyStats, SearchMode};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, params};

/// Windows used by the statistics panel.
const LATENCY_WINDOW_HOURS: i64 = 24;
const USAGE_WINDOW_DAYS: i64 = 30;
/// Rows older than this are pruned on write.
const RETENTION_DAYS: i64 = 31;
const MAX_DURATION_MS: u64 = 3_600_000;
const MAX_RESULT_COUNT: u64 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsagePurpose {
    Answer,
    Chat,
    Embedding,
}

impl UsagePurpose {
    fn tag(self) -> &'static str {
        match self {
            UsagePurpose::Answer => "answer",
            UsagePurpose::Chat => "chat",
            UsagePurpose::Embedding => "embedding",
        }
    }
}

/// Wall-clock latency of one recorded model call. irst_token_ms stays absent
/// when the transport never streamed a token (embeddings, non-streamed calls),
/// and the panel then reports the value as unknown instead of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCallLatency {
    pub duration_ms: u64,
    pub first_token_ms: Option<u64>,
}

/// One model call as its caller measured it. Token counts stay absent when the
/// provider returned none; latency stays absent when nothing was measured, so
/// the panel never reports an unmeasured call as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated: bool,
    pub latency: Option<ModelCallLatency>,
}

/// Wall-clock milliseconds of a measured span, clamped to the documented maximum
/// so an unusually long call still records its usage instead of failing the insert.
pub fn duration_ms(value: std::time::Duration) -> u64 {
    u64::try_from(value.as_millis())
        .unwrap_or(u64::MAX)
        .min(MAX_DURATION_MS)
}

/// Milliseconds elapsed since `started`.
pub fn elapsed_ms(started: std::time::Instant) -> u64 {
    duration_ms(started.elapsed())
}

fn mode_tag(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Lexical => "lexical",
        SearchMode::Semantic => "semantic",
        SearchMode::Hybrid => "hybrid",
    }
}

fn to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::IntegrityFailure)
}

fn prune(db: &Connection, now: DateTime<Utc>) -> Result<()> {
    let cutoff = (now - Duration::days(RETENTION_DAYS)).to_rfc3339();
    db.execute(
        "DELETE FROM retrieval_events WHERE occurred_at<?1",
        [&cutoff],
    )?;
    db.execute(
        "DELETE FROM model_usage_events WHERE occurred_at<?1",
        [&cutoff],
    )?;
    Ok(())
}

impl Library {
    /// Records one completed retrieval. `duration_ms` is measured around the
    /// whole local retrieval by the caller, not around a network round trip.
    pub fn record_retrieval(
        &mut self,
        mode: SearchMode,
        duration_ms: u64,
        result_count: u64,
    ) -> Result<()> {
        if duration_ms > MAX_DURATION_MS || result_count > MAX_RESULT_COUNT {
            return Err(Error::InvalidInput("invalid retrieval metric"));
        }
        let now = Utc::now();
        self.connection.execute(
            "INSERT INTO retrieval_events(occurred_at,mode,duration_ms,result_count) VALUES(?1,?2,?3,?4)",
            params![
                now.to_rfc3339(),
                mode_tag(mode),
                duration_ms as i64,
                result_count as i64
            ],
        )?;
        prune(&self.connection, now)
    }

    /// Records one model call. Absent token counts stay unknown rather than
    /// being reported as zero; `estimated` marks counts derived from text length.
    pub fn record_model_usage(
        &mut self,
        purpose: UsagePurpose,
        provider: &str,
        model: &str,
        usage: ModelUsage,
    ) -> Result<()> {
        let ModelUsage {
            input_tokens,
            output_tokens,
            estimated,
            latency,
        } = usage;
        if provider.is_empty()
            || provider.len() > 64
            || model.is_empty()
            || model.len() > 200
            || input_tokens.is_some_and(|v| v > (1_u64 << 53) - 1)
            || output_tokens.is_some_and(|v| v > (1_u64 << 53) - 1)
        {
            return Err(Error::InvalidInput("invalid usage metric"));
        }
        let (duration_ms, first_token_ms) = match latency {
            // A first token that arrives after the call finished is not a
            // measurement, it is a bug in the caller.
            Some(value)
                if value
                    .first_token_ms
                    .is_some_and(|first| first > value.duration_ms) =>
            {
                return Err(Error::InvalidInput("invalid usage metric"));
            }
            Some(value) => (Some(value.duration_ms), value.first_token_ms),
            None => (None, None),
        };
        let now = Utc::now();
        self.connection.execute(
            "INSERT INTO model_usage_events(occurred_at,purpose,provider,model,input_tokens,output_tokens,estimated,duration_ms,first_token_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                now.to_rfc3339(),
                purpose.tag(),
                provider,
                model,
                input_tokens.map(|value| value as i64),
                output_tokens.map(|value| value as i64),
                i64::from(estimated),
                duration_ms.map(|value| value as i64),
                first_token_ms.map(|value| value as i64)
            ],
        )?;
        prune(&self.connection, now)
    }
}

/// Mean and 95th percentile of successful and failed local retrievals in the
/// last 24 hours. The percentile uses the nearest-rank position on the sorted
/// sample, which is stable for the sample sizes this panel reports.
pub(crate) fn retrieval_latency_stats(db: &Connection) -> Result<RetrievalLatencyStats> {
    let since = (Utc::now() - Duration::hours(LATENCY_WINDOW_HOURS)).to_rfc3339();
    let samples = to_u64(db.query_row(
        "SELECT count(*) FROM retrieval_events WHERE occurred_at>=?1",
        [&since],
        |r| r.get(0),
    )?)?;
    if samples == 0 {
        return Ok(RetrievalLatencyStats::default());
    }
    let average: f64 = db.query_row(
        "SELECT avg(duration_ms) FROM retrieval_events WHERE occurred_at>=?1",
        [&since],
        |r| r.get(0),
    )?;
    let offset = i64::try_from((samples * 95 / 100).min(samples - 1))
        .map_err(|_| Error::IntegrityFailure)?;
    let p95: i64 = db.query_row(
        "SELECT duration_ms FROM retrieval_events WHERE occurred_at>=?1 ORDER BY duration_ms LIMIT 1 OFFSET ?2",
        params![since, offset],
        |r| r.get(0),
    )?;
    Ok(RetrievalLatencyStats {
        samples,
        average_ms: average.max(0.0).round() as u64,
        p95_ms: to_u64(p95)?,
    })
}

pub(crate) fn model_usage_stats(db: &Connection) -> Result<ModelUsageStats> {
    let since = (Utc::now() - Duration::days(USAGE_WINDOW_DAYS)).to_rfc3339();
    let (calls, estimated_calls, without_usage, input, output, timed, duration, first_token): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<f64>,
        Option<f64>,
    ) = db.query_row(
        "SELECT count(*),
                coalesce(sum(estimated),0),
                coalesce(sum(CASE WHEN input_tokens IS NULL AND output_tokens IS NULL THEN 1 ELSE 0 END),0),
                coalesce(sum(coalesce(input_tokens,0)),0),
                coalesce(sum(coalesce(output_tokens,0)),0),
                coalesce(sum(CASE WHEN duration_ms IS NULL THEN 0 ELSE 1 END),0),
                avg(duration_ms),
                avg(first_token_ms)
         FROM model_usage_events WHERE occurred_at>=?1",
        [&since],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        },
    )?;
    Ok(ModelUsageStats {
        calls: to_u64(calls)?,
        estimated_calls: to_u64(estimated_calls)?,
        calls_without_usage: to_u64(without_usage)?,
        input_tokens: to_u64(input)?,
        output_tokens: to_u64(output)?,
        timed_calls: to_u64(timed)?,
        average_duration_ms: average(duration),
        average_first_token_ms: average(first_token),
    })
}

/// Averages stay absent when nothing was measured; an unmeasured call must not
/// be reported as a zero-millisecond call.
fn average(value: Option<f64>) -> Option<u64> {
    value
        .filter(|sample| sample.is_finite() && *sample >= 0.)
        .map(|sample| sample.round() as u64)
}

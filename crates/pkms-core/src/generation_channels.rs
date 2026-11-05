//! Named remote model channels. Settings, revisions and receipts commit together;
//! accepted answer/chat profiles remain independent immutable snapshots.
use crate::*;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeSet;

const MAX_CHANNELS: u32 = 64;
const MAX_MODELS: usize = 128;

pub fn validate_generation_channel_spec(spec: &GenerationChannelSpec) -> Result<()> {
    if spec.name.trim() != spec.name
        || spec.name.is_empty()
        || spec.name.chars().count() > 120
        || spec.name.contains(char::is_control)
        || !matches!(
            spec.provider.as_str(),
            "openai_chat" | "openai_responses" | "anthropic" | "gemini"
        )
        || spec.models.len() > MAX_MODELS
    {
        return Err(Error::InvalidInput("invalid_generation_channel"));
    }
    crate::assistant::validate_remote_generation_endpoint(&spec.endpoint)?;
    if spec.preset == Some(GenerationChannelPreset::CloudflareWorkersAi)
        && (!matches!(spec.provider.as_str(), "openai_chat" | "openai_responses")
            || spec.capabilities.structured_transport == Some(GenerationTransport::Stream)
            || (spec.provider == "openai_responses"
                && spec.capabilities.text_transport != GenerationTransport::Json))
    {
        return Err(Error::InvalidInput("cloudflare_transport_unsupported"));
    }
    let mut models = BTreeSet::new();
    for model in &spec.models {
        if model.model.trim() != model.model || !models.insert(&model.model) {
            return Err(Error::InvalidInput("duplicate_or_invalid_channel_model"));
        }
        validate_generation_profile(&GenerationProfile {
            provider: spec.provider.clone(),
            endpoint: spec.endpoint.clone(),
            model: model.model.clone(),
            model_digest: String::new(),
            config_revision: 1,
            temperature: model.temperature,
            context_tokens: model.context_tokens,
            chat_token_limit: model.chat_token_limit,
            channel_id: None,
            capabilities: None,
        })?;
    }
    Ok(())
}

fn read(db: &Connection, id: &str) -> Result<GenerationChannel> {
    document::valid_uuid(id)?;
    let row = db.query_row(
        "SELECT row_version,spec_json,deleted,created_at,updated_at FROM generation_channels WHERE id=?1",
        [id],
        |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    Ok(GenerationChannel {
        id: id.into(),
        row_version: row.0,
        spec: serde_json::from_str(&row.1)?,
        deleted: row.2,
        created_at: row.3,
        updated_at: row.4,
    })
}

fn settings(db: &Connection, expected: u32) -> Result<GenerationSettings> {
    let settings = crate::assistant::generation_settings(db)?;
    if settings.row_version != expected {
        return Err(Error::MetadataConflict {
            object_id: "generation_settings".into(),
            expected,
            actual: settings.row_version,
        });
    }
    Ok(settings)
}

fn active(channel: &GenerationChannel, expected: u32) -> Result<()> {
    mutations::check_version(&channel.id, expected, channel.row_version)?;
    if channel.deleted {
        return Err(Error::InvalidInput("generation_channel_deleted"));
    }
    Ok(())
}

fn profile(channel: &GenerationChannel, model: &str, revision: u32) -> Result<GenerationProfile> {
    let selected = channel
        .spec
        .models
        .iter()
        .find(|item| item.model == model)
        .ok_or(Error::InvalidInput("generation_channel_model_missing"))?;
    Ok(GenerationProfile {
        provider: channel.spec.provider.clone(),
        endpoint: channel.spec.endpoint.clone(),
        model: selected.model.clone(),
        model_digest: String::new(),
        config_revision: revision,
        temperature: selected.temperature,
        context_tokens: selected.context_tokens,
        chat_token_limit: selected.chat_token_limit,
        channel_id: Some(channel.id.clone()),
        capabilities: Some(channel.spec.capabilities.clone()),
    })
}

fn next_version(version: u32) -> Result<u32> {
    version
        .checked_add(1)
        .ok_or(Error::InvalidInput("generation_settings_version_overflow"))
}

fn write_settings(db: &Connection, settings: &GenerationSettings) -> Result<()> {
    db.execute("UPDATE search_settings SET generation_profile_json=?1,generation_row_version=?2 WHERE singleton=1",
        params![settings.profile.as_ref().map(serde_json::to_string).transpose()?, settings.row_version])?;
    Ok(())
}

fn record(
    db: &Connection,
    command: &str,
    hash: &str,
    kind: &str,
    channel: GenerationChannel,
) -> Result<GenerationChannelMutation> {
    let result = GenerationChannelMutation {
        channel,
        settings: crate::assistant::generation_settings(db)?,
        receipt: mutations::next_receipt(db, command)?,
    };
    mutations::store_receipt(db, hash, kind, &result.channel.id, &result.receipt, &result)?;
    Ok(result)
}

impl Library {
    pub fn generation_channels(&self) -> Result<GenerationChannelsState> {
        let _snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let ids = self
            .connection
            .prepare("SELECT id FROM generation_channels WHERE deleted=0 ORDER BY created_at,id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(GenerationChannelsState {
            channels: ids
                .iter()
                .map(|id| read(&self.connection, id))
                .collect::<Result<_>>()?,
            settings: self.generation_settings()?,
        })
    }

    /// Deleted records remain addressable to explain immutable historical references.
    pub fn generation_channel(&self, id: &str) -> Result<GenerationChannel> {
        read(&self.connection, id)
    }

    pub fn save_generation_channel(
        &mut self,
        command: &str,
        request: SaveGenerationChannel,
    ) -> Result<GenerationChannelMutation> {
        let kind = "generation.channel.save";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(replayed) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(replayed);
        }
        document::valid_uuid(&request.channel_id)?;
        validate_generation_channel_spec(&request.spec)?;
        let mut selected = settings(&tx, request.base_generation_row_version)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM generation_channels WHERE id=?1)",
            [&request.channel_id],
            |row| row.get(0),
        )?;
        let now = Utc::now().to_rfc3339();
        let mut channel = if exists {
            let previous = read(&tx, &request.channel_id)?;
            active(&previous, request.base_row_version)?;
            GenerationChannel {
                row_version: next_version(previous.row_version)?,
                spec: request.spec,
                updated_at: now,
                ..previous
            }
        } else {
            if request.base_row_version != 0 {
                return Err(Error::NotFound);
            }
            let count: u32 = tx.query_row(
                "SELECT count(*) FROM generation_channels WHERE deleted=0",
                [],
                |row| row.get(0),
            )?;
            if count >= MAX_CHANNELS {
                return Err(Error::InvalidInput("generation_channel_limit"));
            }
            GenerationChannel {
                id: request.channel_id,
                row_version: 1,
                spec: request.spec,
                deleted: false,
                created_at: now.clone(),
                updated_at: now,
            }
        };
        if let Some(current) = &selected.profile
            && current.channel_id.as_deref() == Some(channel.id.as_str())
        {
            let revision = next_version(selected.row_version)?;
            selected.profile = Some(profile(&channel, &current.model, revision)?);
            selected.row_version = revision;
            write_settings(&tx, &selected)?;
        }
        tx.execute("INSERT INTO generation_channels(id,row_version,spec_json,deleted,created_at,updated_at) VALUES(?1,?2,?3,0,?4,?5) ON CONFLICT(id) DO UPDATE SET row_version=excluded.row_version,spec_json=excluded.spec_json,updated_at=excluded.updated_at",
            params![channel.id, channel.row_version, serde_json::to_string(&channel.spec)?, channel.created_at, channel.updated_at])?;
        channel.deleted = false;
        let result = record(&tx, command, &hash, kind, channel)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn delete_generation_channel(
        &mut self,
        command: &str,
        request: DeleteGenerationChannel,
    ) -> Result<GenerationChannelMutation> {
        let kind = "generation.channel.delete";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(replayed) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(replayed);
        }
        let mut selected = settings(&tx, request.base_generation_row_version)?;
        let mut channel = read(&tx, &request.channel_id)?;
        active(&channel, request.base_row_version)?;
        if selected
            .profile
            .as_ref()
            .and_then(|profile| profile.channel_id.as_deref())
            == Some(channel.id.as_str())
        {
            if !request.clear_active {
                return Err(Error::InvalidInput("generation_channel_is_active"));
            }
            selected = GenerationSettings {
                profile: None,
                row_version: next_version(selected.row_version)?,
            };
            write_settings(&tx, &selected)?;
        }
        channel.deleted = true;
        channel.row_version = next_version(channel.row_version)?;
        channel.updated_at = Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE generation_channels SET deleted=1,row_version=?2,updated_at=?3 WHERE id=?1",
            params![channel.id, channel.row_version, channel.updated_at],
        )?;
        let result = record(&tx, command, &hash, kind, channel)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn select_generation_channel_model(
        &mut self,
        command: &str,
        request: SelectGenerationChannelModel,
    ) -> Result<GenerationSettingsMutation> {
        let kind = "generation.channel.select";
        let hash = mutations::request_hash(command, kind, &request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(replayed) = mutations::replay(&tx, command, &hash, kind)? {
            return Ok(replayed);
        }
        let previous = settings(&tx, request.base_generation_row_version)?;
        let channel = read(&tx, &request.channel_id)?;
        active(&channel, request.base_row_version)?;
        let version = next_version(previous.row_version)?;
        let selected = GenerationSettings {
            profile: Some(profile(&channel, &request.model, version)?),
            row_version: version,
        };
        write_settings(&tx, &selected)?;
        let result = GenerationSettingsMutation {
            settings: selected,
            receipt: mutations::next_receipt(&tx, command)?,
        };
        mutations::store_receipt(
            &tx,
            &hash,
            kind,
            "generation_settings",
            &result.receipt,
            &result,
        )?;
        tx.commit()?;
        Ok(result)
    }
}

pub(crate) fn validate_backup(db: &Connection) -> Result<()> {
    let ids = db
        .prepare("SELECT id FROM generation_channels")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut active_count = 0;
    for id in ids {
        let channel = read(db, &id)?;
        validate_generation_channel_spec(&channel.spec).map_err(|_| Error::IntegrityFailure)?;
        let created = chrono::DateTime::parse_from_rfc3339(&channel.created_at)
            .map_err(|_| Error::IntegrityFailure)?;
        let updated = chrono::DateTime::parse_from_rfc3339(&channel.updated_at)
            .map_err(|_| Error::IntegrityFailure)?;
        if updated < created || channel.row_version == 0 {
            return Err(Error::IntegrityFailure);
        }
        if !channel.deleted {
            active_count += 1;
        }
    }
    if active_count > MAX_CHANNELS {
        return Err(Error::IntegrityFailure);
    }
    let settings = crate::assistant::generation_settings(db)?;
    if let Some(selected) = &settings.profile
        && let Some(channel_id) = &selected.channel_id
    {
        let channel = read(db, channel_id)?;
        if channel.deleted
            || serde_json::to_value(selected)?
                != serde_json::to_value(profile(&channel, &selected.model, settings.row_version)?)?
        {
            return Err(Error::IntegrityFailure);
        }
    }
    Ok(())
}

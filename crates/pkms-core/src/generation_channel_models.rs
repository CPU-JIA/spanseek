use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GenerationTransport {
    Stream,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationCapabilities {
    pub text_transport: GenerationTransport,
    /// None explicitly means that evidence answers requiring structured output are unsupported.
    pub structured_transport: Option<GenerationTransport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum GenerationChannelPreset {
    CloudflareWorkersAi,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationChannelModel {
    pub model: String,
    pub context_tokens: u32,
    pub temperature: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub chat_token_limit: Option<crate::ChatTokenLimit>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationChannelSpec {
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    /// An empty list allows configuring credentials before discovering or adding models.
    pub models: Vec<GenerationChannelModel>,
    pub capabilities: GenerationCapabilities,
    pub preset: Option<GenerationChannelPreset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationChannel {
    pub id: String,
    pub row_version: u32,
    pub spec: GenerationChannelSpec,
    pub deleted: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationChannelsState {
    pub channels: Vec<GenerationChannel>,
    pub settings: crate::GenerationSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredGenerationModel {
    pub model: String,
    pub display_name: Option<String>,
    pub context_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RemoteGenerationModels {
    pub channel_id: String,
    pub row_version: u32,
    pub provider: String,
    pub endpoint: String,
    pub models: Vec<DiscoveredGenerationModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SaveGenerationChannel {
    pub channel_id: String,
    /// Zero creates a new stable ID; existing and deleted IDs cannot be reused.
    pub base_row_version: u32,
    pub base_generation_row_version: u32,
    pub spec: GenerationChannelSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DeleteGenerationChannel {
    pub channel_id: String,
    pub base_row_version: u32,
    pub base_generation_row_version: u32,
    pub clear_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SelectGenerationChannelModel {
    pub channel_id: String,
    pub model: String,
    pub base_row_version: u32,
    pub base_generation_row_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct GenerationChannelMutation {
    pub channel: GenerationChannel,
    pub settings: crate::GenerationSettings,
    pub receipt: crate::MetadataReceipt,
}

impl crate::mutations::RecordedResult for GenerationChannelMutation {
    fn receipt_mut(&mut self) -> &mut crate::MetadataReceipt {
        &mut self.receipt
    }
}

//! Gateway, model metadata, and input-token operation workflows.

use super::*;

pub(in crate::operation) const SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE: &str = ".";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Source of a pre-query input-token count.
pub enum InputTokenCountSource {
    PrequeryInput,
    PrequerySyntheticInput,
}

impl InputTokenCountSource {
    /// Returns the stable API/UI label for this count source.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrequeryInput => "prequery_input",
            Self::PrequerySyntheticInput => "prequery_synthetic_input",
        }
    }
}

/// Read-only model-facing payload pieces prepared for input-token counting.
///
/// Loading these pieces is separate from the async Bifrost request so API
/// handlers can release SQLite state before awaiting network I/O.
pub struct InputTokenCountContext {
    pub(in crate::operation) model_messages: Vec<Message>,
    pub(in crate::operation) tool_schemas: Vec<ToolSchema>,
    source: InputTokenCountSource,
}

impl InputTokenCountContext {
    /// Returns whether the count uses real context input or synthetic input.
    pub fn source(&self) -> InputTokenCountSource {
        self.source
    }
}

#[derive(Debug, Serialize)]
/// Normalized model-parameter metadata used by developer clients.
///
/// Bifrost returns a richer raw parameter schema. Windie extracts only the
/// effort selector needed for runtime query controls and preserves the raw
/// response for inspection/debugging.
pub struct ModelRuntimeParameters {
    model: String,
    supports_reasoning: bool,
    supports_prompt_caching: bool,
    reasoning: Option<ReasoningParameter>,
    raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Effort selector derived from Bifrost model parameters.
pub struct ReasoningParameter {
    source: ReasoningParameterSource,
    options: Vec<ModelParameterOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Bifrost parameter source used to build a normalized effort selector.
pub enum ReasoningParameterSource {
    ReasoningEffort,
    OutputConfigEffort,
}
/// Returns whether the configured local Bifrost gateway is running.
pub async fn gateway_status(gateway_url: GatewayUrl) -> bool {
    BifrostGateway::new(gateway_url).is_running().await
}

/// Starts the configured local Bifrost gateway if it is not already running.
pub async fn start_gateway(gateway_url: GatewayUrl) -> Result<GatewayStart> {
    BifrostGateway::new(gateway_url).start().await
}

/// Stops the configured local Bifrost gateway when Windie can identify it.
pub async fn stop_gateway(gateway_url: GatewayUrl) -> Result<GatewayStop> {
    BifrostGateway::new(gateway_url).stop().await
}

/// Requires the configured local Bifrost gateway to be reachable.
pub async fn require_gateway_running(gateway_url: GatewayUrl) -> Result<()> {
    BifrostGateway::new(gateway_url).require_running().await
}

/// Lists models from the currently running Bifrost gateway.
///
/// This operation is intentionally read-only. It does not start, stop, restart,
/// or reconfigure Bifrost; users restart the gateway explicitly after changing
/// `.env`.
pub async fn list_models(gateway_url: GatewayUrl, base_url: BaseUrl) -> Result<Vec<ModelInfo>> {
    require_gateway_running(gateway_url).await?;

    llm::list_models(base_url).await
}

/// Loads model-parameter metadata for one selected model.
///
/// This keeps Bifrost as the source of model capability truth. Windie only
/// normalizes Bifrost's effort parameter into the small shape the inspector
/// needs to render the reasoning dropdown.
pub async fn model_runtime_parameters(
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: &ModelName,
) -> Result<ModelRuntimeParameters> {
    require_gateway_running(gateway_url).await?;

    let parameters = llm::model_parameters(base_url, model).await?;
    let reasoning = reasoning_parameter(&parameters.model_parameters);

    Ok(ModelRuntimeParameters {
        model: model.as_str().to_string(),
        supports_reasoning: parameters.supports_reasoning.unwrap_or(false) || reasoning.is_some(),
        supports_prompt_caching: parameters.supports_prompt_caching.unwrap_or(false),
        reasoning,
        raw: parameters.raw,
    })
}

/// Extracts an effort selector from Bifrost model-parameter metadata.
fn reasoning_parameter(parameters: &[ModelParameter]) -> Option<ReasoningParameter> {
    parameters
        .iter()
        .find(|parameter| parameter.id == "reasoning_effort" && !parameter.options.is_empty())
        .map(|parameter| ReasoningParameter {
            source: ReasoningParameterSource::ReasoningEffort,
            options: parameter.options.clone(),
        })
        .or_else(|| {
            parameters
                .iter()
                .find(|parameter| {
                    parameter.id == "output_config"
                        && parameter.accessor_key.as_deref() == Some("effort")
                        && !parameter.options.is_empty()
                })
                .map(|parameter| ReasoningParameter {
                    source: ReasoningParameterSource::OutputConfigEffort,
                    options: parameter.options.clone(),
                })
        })
}

/// Builds an optional provider prompt-cache request for one conversation turn.
///
/// Bifrost owns model capability metadata. Windie asks for that metadata before
/// a query and only creates a cache hint when the selected model explicitly
/// reports prompt-cache support. Metadata lookup failure is treated as
/// unsupported so prompt caching remains additive and does not block normal
/// queries for custom or older Bifrost model entries.
pub(super) async fn prompt_cache_request(
    base_url: BaseUrl,
    model: &ModelName,
    conversation_id: &ConversationId,
) -> Option<PromptCacheRequest> {
    let parameters = llm::model_parameters(base_url, model).await.ok()?;
    if !parameters.supports_prompt_caching.unwrap_or(false) {
        return None;
    }

    Some(conversation_prompt_cache_request(conversation_id))
}

/// Creates the stable prompt-cache identity for one Windie conversation.
pub(in crate::operation) fn conversation_prompt_cache_request(
    conversation_id: &ConversationId,
) -> PromptCacheRequest {
    PromptCacheRequest {
        key: format!("windie:{}", conversation_id.as_str()),
        retention: Some("24h".to_string()),
    }
}

/// Builds the current model-facing input-token context for one conversation.
///
/// This is a read-only preview operation. It builds the same flattened context
/// and attached tool schema list used by query execution, but it does not run
/// query preparation because that path can persist automatic tool results.
/// Bifrost requires at least one Responses input item before it can count tool
/// schema tokens, so a tool-only setup uses a tiny synthetic system message
/// that is never persisted and never sent during a real query.
pub fn conversation_input_token_context(
    store: &Store,
    conversation_id: &ConversationId,
    head_message_id: Option<&MessageId>,
) -> Result<Option<InputTokenCountContext>> {
    let model_context =
        ContextBuilder::build_model_context(store, conversation_id, head_message_id)?;
    let mut model_messages = model_context.messages;
    let tool_schemas = model_context.tool_schemas;
    let source = if model_messages.is_empty() {
        if tool_schemas.is_empty() {
            return Ok(None);
        }
        model_messages.push(synthetic_input_token_count_message());
        InputTokenCountSource::PrequerySyntheticInput
    } else {
        InputTokenCountSource::PrequeryInput
    };

    Ok(Some(InputTokenCountContext {
        model_messages,
        tool_schemas,
        source,
    }))
}

/// Builds the tiny provider input needed to count a tool-only setup.
fn synthetic_input_token_count_message() -> Message {
    Message {
        id: None,
        parent_message_id: None,
        role: Role::System,
        content: SYNTHETIC_INPUT_TOKEN_COUNT_MESSAGE.to_string(),
        parts: Vec::new(),
        metadata: None,
    }
}

/// Counts prepared model-facing input tokens through Bifrost.
pub async fn count_input_tokens_for_context(
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: &ModelName,
    context: Option<InputTokenCountContext>,
) -> Result<Option<InputTokenCount>> {
    let Some(context) = context else {
        return Ok(None);
    };
    require_gateway_running(gateway_url).await?;

    let client = BifrostClient::new(base_url, model.clone());
    client
        .count_input_tokens(&context.model_messages, &context.tool_schemas)
        .await
}

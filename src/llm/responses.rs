//! OpenAI-compatible Responses wire structs.

use serde::{Deserialize, Serialize};

use super::serialization::ReasoningRequest;

#[derive(Debug, Serialize)]
/// JSON request body sent to `/responses`.
pub(super) struct ResponsesRequest<'a> {
    pub(super) model: &'a str,
    pub(super) input: Vec<ResponsesInputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<ResponsesTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reasoning: Option<&'a ReasoningRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) prompt_cache_retention: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_control: Option<CacheControlRequest>,
    pub(super) stream: bool,
}

#[derive(Debug, Serialize)]
/// Anthropic-family prompt-cache control forwarded through Bifrost.
pub(super) struct CacheControlRequest {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
}

#[derive(Debug)]
/// Provider-specific cache fields to include in one Responses request.
pub(super) struct PromptCacheFields<'a> {
    pub(super) prompt_cache_key: Option<&'a str>,
    pub(super) prompt_cache_retention: Option<&'a str>,
    pub(super) cache_control: Option<CacheControlRequest>,
}

#[derive(Debug, Serialize)]
/// JSON request body sent to `/responses/input_tokens`.
pub(super) struct ResponsesInputTokensRequest<'a> {
    pub(super) model: &'a str,
    pub(super) input: Vec<ResponsesInputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<ResponsesTool<'a>>>,
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One item inside a Responses `input` array.
pub(super) enum ResponsesInputItem<'a> {
    Message(ResponsesMessageItem<'a>),
    FunctionCall(ResponsesFunctionCallItem<'a>),
    FunctionCallOutput(ResponsesFunctionCallOutputItem<'a>),
}

#[derive(Debug, Serialize)]
/// User/system/assistant message item for Responses input.
pub(super) struct ResponsesMessageItem<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) role: &'static str,
    pub(super) content: ResponsesMessageContent<'a>,
}

#[derive(Debug, Serialize)]
/// Assistant function-call item for Responses input history.
pub(super) struct ResponsesFunctionCallItem<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) call_id: &'a str,
    pub(super) name: &'a str,
    pub(super) arguments: &'a str,
    pub(super) status: &'static str,
}

#[derive(Debug, Serialize)]
/// Function-call output item for Responses input history.
pub(super) struct ResponsesFunctionCallOutputItem<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) call_id: &'a str,
    pub(super) output: ResponsesToolOutput<'a>,
    pub(super) status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// Responses message content: plain text or ordered multimodal blocks.
pub(super) enum ResponsesMessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ResponsesContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// Responses function-call output: plain text or ordered multimodal blocks.
pub(super) enum ResponsesToolOutput<'a> {
    Text(&'a str),
    Parts(Vec<ResponsesContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One Responses content block.
pub(super) enum ResponsesContentPart<'a> {
    Text(ResponsesTextPart<'a>),
    Image(ResponsesImagePart),
}

#[derive(Debug, Serialize)]
/// Responses text content block.
pub(super) struct ResponsesTextPart<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) text: &'a str,
}

#[derive(Debug, Serialize)]
/// Responses image content block.
pub(super) struct ResponsesImagePart {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) image_url: String,
    pub(super) detail: &'static str,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible function tool definition sent through Responses.
pub(super) struct ResponsesTool<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'static str,
    pub(super) name: &'a str,
    pub(super) description: &'a str,
    pub(super) parameters: &'a serde_json::Value,
}

#[derive(Debug, Deserialize)]
/// One streamed Responses server-sent event payload from Bifrost.
pub(super) struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) output_index: Option<u16>,
    pub(super) delta: Option<String>,
    pub(super) text: Option<String>,
    pub(super) refusal: Option<String>,
    pub(super) arguments: Option<String>,
    pub(super) item: Option<ResponsesStreamItem>,
    pub(super) response: Option<ResponsesStreamResponse>,
    pub(super) error: Option<ResponsesStreamError>,
    pub(super) message: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Stream item used by `response.output_item.*` events.
pub(super) struct ResponsesStreamItem {
    pub(super) id: Option<String>,
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    pub(super) call_id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Terminal response body used by failed/incomplete Responses events.
pub(super) struct ResponsesStreamResponse {
    pub(super) error: Option<ResponsesStreamError>,
    pub(super) usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
/// Error payload embedded in Responses stream events.
pub(super) struct ResponsesStreamError {
    pub(super) message: Option<String>,
    #[serde(rename = "type")]
    pub(super) kind: Option<String>,
    pub(super) code: Option<String>,
}

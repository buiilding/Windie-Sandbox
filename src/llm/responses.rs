//! OpenAI-compatible Responses request and wire serialization.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;

use super::{
    Message, ModelName, PromptCacheRequest, ReasoningRequest, ToolSchema,
    provider_local_model_name, provider_name,
};
use crate::conversation::{ImagePart, MessagePart};

#[derive(Debug, Serialize)]
/// JSON request body sent to `/responses`.
pub(super) struct ResponsesRequest<'a> {
    model: &'a str,
    input: Vec<ResponsesInputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<&'a ReasoningRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_retention: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControlRequest>,
    stream: bool,
}

#[derive(Debug, Serialize)]
/// Anthropic-family prompt-cache control forwarded through Bifrost.
struct CacheControlRequest {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Debug)]
/// Provider-specific cache fields to include in one Responses request.
struct PromptCacheFields<'a> {
    prompt_cache_key: Option<&'a str>,
    prompt_cache_retention: Option<&'a str>,
    cache_control: Option<CacheControlRequest>,
}

#[derive(Debug, Serialize)]
/// JSON request body sent to `/responses/input_tokens`.
pub(super) struct ResponsesInputTokensRequest<'a> {
    model: &'a str,
    input: Vec<ResponsesInputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool<'a>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Model-facing image detail level serialized onto every Responses image block.
///
/// Windie stores user images and tool-output images through the same
/// `MessagePart::Image` primitive. Choosing the detail level here keeps visual
/// grounding policy inside the provider HTTP boundary instead of duplicating it
/// across input, MCP, or tool-provider code paths.
enum ImageInputDetail {
    High,
    Original,
}

impl ImageInputDetail {
    /// Returns the OpenAI-compatible wire value for Responses `input_image`.
    fn as_wire_value(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Original => "original",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One item inside a Responses `input` array.
enum ResponsesInputItem<'a> {
    Message(ResponsesMessageItem<'a>),
    FunctionCall(ResponsesFunctionCallItem<'a>),
    FunctionCallOutput(ResponsesFunctionCallOutputItem<'a>),
}

#[derive(Debug, Serialize)]
/// User/system/assistant message item for Responses input.
struct ResponsesMessageItem<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    role: &'static str,
    content: ResponsesMessageContent<'a>,
}

#[derive(Debug, Serialize)]
/// Assistant function-call item for Responses input history.
struct ResponsesFunctionCallItem<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    name: &'a str,
    arguments: &'a str,
    status: &'static str,
}

#[derive(Debug, Serialize)]
/// Function-call output item for Responses input history.
struct ResponsesFunctionCallOutputItem<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    call_id: &'a str,
    output: ResponsesToolOutput<'a>,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// Responses message content: plain text or ordered multimodal blocks.
enum ResponsesMessageContent<'a> {
    Text(&'a str),
    Parts(Vec<ResponsesContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// Responses function-call output: plain text or ordered multimodal blocks.
enum ResponsesToolOutput<'a> {
    Text(&'a str),
    Parts(Vec<ResponsesContentPart<'a>>),
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
/// One Responses content block.
enum ResponsesContentPart<'a> {
    Text(ResponsesTextPart<'a>),
    Image(ResponsesImagePart),
}

#[derive(Debug, Serialize)]
/// Responses text content block.
struct ResponsesTextPart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    text: &'a str,
}

#[derive(Debug, Serialize)]
/// Responses image content block.
struct ResponsesImagePart {
    #[serde(rename = "type")]
    kind: &'static str,
    image_url: String,
    detail: &'static str,
}

#[derive(Debug, Serialize)]
/// OpenAI-compatible function tool definition sent through Responses.
struct ResponsesTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

fn prompt_cache_fields<'a>(
    model: &str,
    prompt_cache: Option<&'a PromptCacheRequest>,
) -> PromptCacheFields<'a> {
    let Some(prompt_cache) = prompt_cache else {
        return PromptCacheFields {
            prompt_cache_key: None,
            prompt_cache_retention: None,
            cache_control: None,
        };
    };

    match provider_name(model) {
        Some("openai") => PromptCacheFields {
            prompt_cache_key: Some(prompt_cache.key.as_str()),
            prompt_cache_retention: prompt_cache.retention.as_deref(),
            cache_control: None,
        },
        Some("anthropic") => PromptCacheFields {
            prompt_cache_key: None,
            prompt_cache_retention: None,
            cache_control: Some(CacheControlRequest { kind: "ephemeral" }),
        },
        _ => PromptCacheFields {
            prompt_cache_key: None,
            prompt_cache_retention: None,
            cache_control: None,
        },
    }
}

/// Returns the provider prefix from a Bifrost model id such as `openai/gpt-5.5`.
/// Chooses the Responses image detail level for one concrete Bifrost model.
///
/// `high` is the provider-unified default because it is broadly understood by
/// OpenAI-compatible vision adapters. `original` is reserved for known OpenAI
/// model names where OpenAI documents pixel-preserving image processing for
/// GUI grounding and computer-use accuracy.
fn image_input_detail_for_model(model: &str) -> ImageInputDetail {
    if provider_name(model) == Some("openai")
        && openai_model_supports_original_image_detail(provider_local_model_name(model))
    {
        return ImageInputDetail::Original;
    }

    ImageInputDetail::High
}

/// Returns whether one OpenAI-local model name supports `detail: original`.
fn openai_model_supports_original_image_detail(model: &str) -> bool {
    let model = model.to_ascii_lowercase();

    if model.starts_with("gpt-5.4-mini") || model.starts_with("gpt-5.4-nano") {
        return false;
    }

    model == "gpt-5.4"
        || model.starts_with("gpt-5.4-")
        || model.starts_with("gpt-5.5")
        || model.starts_with("gpt-5.6")
}

/// Converts Windie's internal messages into the Responses request input array.
fn responses_input(
    messages: &[Message],
    image_detail: ImageInputDetail,
) -> Vec<ResponsesInputItem<'_>> {
    messages
        .iter()
        .flat_map(|message| responses_items_for_message(message, image_detail))
        .collect()
}

/// Converts one Windie message into one or more Responses input items.
fn responses_items_for_message(
    message: &Message,
    image_detail: ImageInputDetail,
) -> Vec<ResponsesInputItem<'_>> {
    match message.role {
        crate::conversation::Role::Assistant => {
            let metadata = message.metadata.as_ref();
            if let Some(tool_calls) = metadata
                .map(|metadata| metadata.tool_calls.as_slice())
                .filter(|tool_calls| !tool_calls.is_empty())
            {
                let mut items = Vec::new();
                if !message.content.is_empty() || !message.parts.is_empty() {
                    items.push(ResponsesInputItem::Message(ResponsesMessageItem {
                        kind: "message",
                        role: "assistant",
                        content: responses_message_content(message, image_detail),
                    }));
                }
                items.extend(
                    tool_calls
                        .iter()
                        .map(|tool_call| {
                            ResponsesInputItem::FunctionCall(ResponsesFunctionCallItem {
                                kind: "function_call",
                                call_id: tool_call.id.as_str(),
                                name: tool_call.name(),
                                arguments: tool_call.arguments(),
                                status: "completed",
                            })
                        })
                        .collect::<Vec<_>>(),
                );

                return items;
            }

            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "assistant",
                content: responses_message_content(message, image_detail),
            })]
        }
        crate::conversation::Role::Tool => {
            let call_id = message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.tool_call_id.as_ref())
                .map(|id| id.as_str());
            call_id
                .map(|call_id| {
                    vec![ResponsesInputItem::FunctionCallOutput(
                        ResponsesFunctionCallOutputItem {
                            kind: "function_call_output",
                            call_id,
                            output: responses_tool_output(message, image_detail),
                            status: "completed",
                        },
                    )]
                })
                .unwrap_or_default()
        }
        crate::conversation::Role::System => {
            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "system",
                content: responses_message_content(message, image_detail),
            })]
        }
        crate::conversation::Role::User => {
            vec![ResponsesInputItem::Message(ResponsesMessageItem {
                kind: "message",
                role: "user",
                content: responses_message_content(message, image_detail),
            })]
        }
    }
}

/// Converts one normal message body into Responses content.
fn responses_message_content(
    message: &Message,
    image_detail: ImageInputDetail,
) -> ResponsesMessageContent<'_> {
    if message.parts.is_empty() {
        if message.role == crate::conversation::Role::Assistant && !message.content.is_empty() {
            return ResponsesMessageContent::Parts(vec![ResponsesContentPart::Text(
                ResponsesTextPart {
                    kind: "output_text",
                    text: &message.content,
                },
            )]);
        }

        return ResponsesMessageContent::Text(&message.content);
    }

    ResponsesMessageContent::Parts(responses_content_parts(
        &message.parts,
        message.role == crate::conversation::Role::Assistant,
        image_detail,
    ))
}

/// Converts one tool message body into Responses function-call output.
fn responses_tool_output(
    message: &Message,
    image_detail: ImageInputDetail,
) -> ResponsesToolOutput<'_> {
    if message.parts.is_empty() {
        return ResponsesToolOutput::Text(&message.content);
    }

    ResponsesToolOutput::Parts(responses_content_parts(&message.parts, false, image_detail))
}

/// Converts stored text/image parts into Responses content blocks.
fn responses_content_parts(
    parts: &[MessagePart],
    assistant_output: bool,
    image_detail: ImageInputDetail,
) -> Vec<ResponsesContentPart<'_>> {
    let text_kind = if assistant_output {
        "output_text"
    } else {
        "input_text"
    };

    parts
        .iter()
        .map(|part| match part {
            MessagePart::Text(text) => ResponsesContentPart::Text(ResponsesTextPart {
                kind: text_kind,
                text,
            }),
            MessagePart::Image(image) => {
                ResponsesContentPart::Image(responses_image_part(image, image_detail))
            }
        })
        .collect()
}

/// Encodes one persisted image as the data URL accepted by Responses.
fn responses_image_part(image: &ImagePart, detail: ImageInputDetail) -> ResponsesImagePart {
    ResponsesImagePart {
        kind: "input_image",
        image_url: format!(
            "data:{};base64,{}",
            image.mime_type,
            STANDARD.encode(&image.bytes)
        ),
        detail: detail.as_wire_value(),
    }
}

/// Converts Windie's tool schemas into Responses function tool definitions.
fn responses_tools(tools: &[ToolSchema]) -> Option<Vec<ResponsesTool<'_>>> {
    if tools.is_empty() {
        return None;
    }

    Some(
        tools
            .iter()
            .map(|tool| ResponsesTool {
                kind: "function",
                name: tool.name.as_str(),
                description: tool.description.as_str(),
                parameters: &tool.parameters,
            })
            .collect(),
    )
}

/// Builds the JSON body for a streaming Responses request.
pub(super) fn responses_request<'a>(
    model: &'a ModelName,
    messages: &'a [Message],
    tools: &'a [ToolSchema],
    reasoning: Option<&'a ReasoningRequest>,
    prompt_cache: Option<&'a PromptCacheRequest>,
) -> ResponsesRequest<'a> {
    let prompt_cache_fields = prompt_cache_fields(model.as_str(), prompt_cache);
    ResponsesRequest {
        model: model.as_str(),
        input: responses_input(messages, image_input_detail_for_model(model.as_str())),
        tools: responses_tools(tools),
        reasoning,
        prompt_cache_key: prompt_cache_fields.prompt_cache_key,
        prompt_cache_retention: prompt_cache_fields.prompt_cache_retention,
        cache_control: prompt_cache_fields.cache_control,
        stream: true,
    }
}

/// Builds the JSON body for a Responses input-token request.
pub(super) fn input_tokens_request<'a>(
    model: &'a ModelName,
    messages: &'a [Message],
    tools: &'a [ToolSchema],
) -> ResponsesInputTokensRequest<'a> {
    ResponsesInputTokensRequest {
        model: model.as_str(),
        input: responses_input(messages, image_input_detail_for_model(model.as_str())),
        tools: responses_tools(tools),
    }
}

#[cfg(test)]
mod tests;

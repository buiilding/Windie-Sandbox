//! Serialization from Windie messages and tools into Responses wire values.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

use crate::conversation::{ImagePart, Message, MessagePart};
use crate::tool::ToolSchema;

use super::responses::{
    CacheControlRequest, PromptCacheFields, ResponsesContentPart, ResponsesFunctionCallItem,
    ResponsesFunctionCallOutputItem, ResponsesImagePart, ResponsesInputItem,
    ResponsesMessageContent, ResponsesMessageItem, ResponsesTextPart, ResponsesTool,
    ResponsesToolOutput,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Optional normalized reasoning controls sent to Bifrost.
///
/// Windie stores no model-specific reasoning table. Clients choose a value that
/// came from Bifrost model-parameter metadata, and `llm.rs` serializes it into
/// the OpenAI-compatible `reasoning` object Bifrost already understands.
pub struct ReasoningRequest {
    /// Model-specific reasoning effort selected by the user/client.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Optional visible reasoning-summary mode for OpenAI Responses models.
    ///
    /// This is separate from `effort`: a model can spend hidden reasoning
    /// tokens without returning displayable summary text unless this field is
    /// requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl ReasoningRequest {
    /// Returns whether this request would serialize any provider-facing data.
    pub fn is_empty(&self) -> bool {
        self.effort.is_none() && self.summary.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Provider prompt-cache hint for one model request.
///
/// Windie owns conversation identity, so it creates the stable cache key. The
/// provider-specific wire mapping stays in this module: OpenAI receives
/// `prompt_cache_key` fields, while Anthropic receives `cache_control`.
pub struct PromptCacheRequest {
    /// Stable provider cache key for the repeated prompt prefix.
    pub key: String,
    /// Optional provider retention hint. OpenAI-compatible providers use this;
    /// Anthropic cache-control markers ignore it.
    pub retention: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Model-facing image detail level serialized onto every Responses image block.
///
/// Windie stores user images and tool-output images through the same
/// `MessagePart::Image` primitive. Choosing the detail level here keeps visual
/// grounding policy inside the provider HTTP boundary instead of duplicating it
/// across input, MCP, or tool-provider code paths.
pub(super) enum ImageInputDetail {
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
/// Builds provider-specific prompt-cache fields for Bifrost's Responses route.
///
/// OpenAI and Anthropic expose different cache controls. Windie keeps one
/// internal cache request and lets this provider HTTP boundary translate it.
/// Unqualified or unsupported provider names intentionally serialize no cache
/// fields because Windie cannot know the correct upstream contract.
pub(super) fn prompt_cache_fields<'a>(
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
fn provider_name(model: &str) -> Option<&str> {
    model.split_once('/').map(|(provider, _)| provider)
}

/// Chooses the Responses image detail level for one concrete Bifrost model.
///
/// `high` is the provider-unified default because it is broadly understood by
/// OpenAI-compatible vision adapters. `original` is reserved for known OpenAI
/// model names where OpenAI documents pixel-preserving image processing for
/// GUI grounding and computer-use accuracy.
pub(super) fn image_input_detail_for_model(model: &str) -> ImageInputDetail {
    if provider_name(model) == Some("openai")
        && openai_model_supports_original_image_detail(provider_local_image_model_name(model))
    {
        return ImageInputDetail::Original;
    }

    ImageInputDetail::High
}

/// Returns the final local model segment for provider-specific image handling.
fn provider_local_image_model_name(model: &str) -> &str {
    model
        .rsplit_once('/')
        .map(|(_, local_model)| local_model)
        .unwrap_or(model)
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
pub(super) fn responses_input(
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
pub(super) fn responses_tools(tools: &[ToolSchema]) -> Option<Vec<ResponsesTool<'_>>> {
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

# Model Parameters and Reasoning

Windie asks Bifrost for model capability metadata instead of maintaining its
own model-specific reasoning and prompt-cache tables.

## Two Model Endpoints

Windie uses two distinct Bifrost surfaces:

1. the OpenAI-compatible model list for model IDs and token limits;
2. Bifrost's management parameter endpoint for capability and option metadata.

The inspector calls Windie's corresponding routes:

```text
GET /api/models
GET /api/model-parameters?model=<provider/model>
```

## Model Catalog

The model list is normalized to:

- model ID;
- context length;
- maximum input tokens;
- maximum output tokens.

The inspector uses IDs for selection and token limits for the context meter.
Windie does not infer reasoning support from model names in this catalog.

## Parameter Lookup Names

Bifrost's parameter datasheet may identify a model differently from its routed
request name. For `provider/namespace/model`, Windie tries distinct forms from
most to least specific:

```text
provider/namespace/model
namespace/model
model
```

For the more common `provider/model`, it tries:

```text
provider/model
model
```

A `404` advances to the next form. The first successful response is decoded.
Other non-success statuses fail the operation.

## Normalized Capability Result

Windie's API returns:

- requested model ID;
- `supports_reasoning`;
- `supports_prompt_caching`;
- optional normalized reasoning selector;
- the complete raw Bifrost metadata.

If no parameter record is found, Windie returns both support flags as false,
no reasoning selector, and null raw metadata.

## Reasoning Parameter Extraction

Windie looks for one of two Bifrost parameter shapes:

1. parameter ID `reasoning_effort` with non-empty options;
2. parameter ID `output_config`, accessor key `effort`, and non-empty options.

The normalized selector preserves each option's value and label and records
which source shape was used.

Reasoning is considered supported when Bifrost explicitly says so or when one
of these selectors exists.

Windie currently does not expose every arbitrary model parameter in the
inspector. It preserves the raw response for inspection but normalizes only the
controls runtime presently uses.

## Inspector Parameter Cache

The inspector fetches parameters only when:

- the gateway is running;
- model loading has completed without error;
- the selected model exists in the loaded catalog.

State is cached by model ID as `loading`, `ready`, or `error`. A ready result is
reused. An error is retained rather than retried on every render.

The composer shows its reasoning selector only when normalized options exist.
If a previously selected effort is not valid for the new model's options, the
composer clears that selection through the conversation reasoning API.

## Persisted Model

Every conversation stores a default model. Setting a new model validates
non-empty text, persists it, and clears the conversation's reasoning effort so
an option from the old model cannot silently carry to the new model.

A query or inspection request can also supply a model override. That override
applies only to the current operation and does not change the persisted model.

## Persisted Reasoning

The conversation stores only an optional reasoning effort string. Loading it
produces a normalized request object:

```json
{"effort":"high","summary":null}
```

Setting `null` or empty reasoning clears the stored effort. The backend trims
persisted effort text.

A query may provide a complete one-request reasoning object. When present, it
takes precedence over conversation reasoning without being persisted.

The current CLI exposes neither persisted reasoning mutation nor a query
reasoning override. The inspector/API own those controls today.

## OpenAI Reasoning Summary

Before a model request, Windie applies a small provider-boundary normalization.
For a routed OpenAI model, when reasoning effort exists and no summary mode was
specified, Windie adds:

```json
{"summary":"auto"}
```

This requests visible reasoning-summary deltas. Explicit summary choices are
preserved. Non-OpenAI providers do not receive this added summary field.

Returned reasoning is accumulated separately from assistant text and persisted
in assistant metadata. Structured provider reasoning details are also retained
when supplied.

## Reasoning and Tools

Bifrost metadata may report both general reasoning support and reasoning with
tool-call support. Windie preserves both in the raw/typed model parameter data,
but the inspector's normalized `supports_reasoning` flag currently focuses on
whether reasoning controls should be shown.

Policy and tool execution do not interpret reasoning content.

## Prompt Cache Capability

At the start of each runtime operation, Windie loads parameters for the model
captured in that operation's immutable configuration snapshot. If
`supports_prompt_caching` is true, it creates a stable key:

```text
windie:<conversation_id>
```

The internal request asks for 24-hour retention. The LLM boundary maps it to:

- OpenAI prompt-cache key and retention fields;
- Anthropic ephemeral cache control;
- no fields for unknown providers.

Parameter lookup failure is treated as no prompt-cache support and does not
block the model query.

The prompt-cache capability result is fixed for that operation but is not
persisted on the conversation.

## Ownership Boundary

Bifrost owns capability truth and parameter metadata. Windie owns:

- trying the relevant model identity forms;
- extracting the reasoning selector used by its clients;
- persisting the user's conversation-level effort choice;
- converting normalized choices into the Responses request.

## Relevant Code

- `src/llm.rs`
- `src/operation.rs`
- `src/store.rs`
- `src/api.rs`
- `dev/windie-inspector/src/context/WindieContext.jsx`
- `dev/windie-inspector/src/components/windie/Composer.jsx`

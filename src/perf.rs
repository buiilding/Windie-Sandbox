//! Performance baseline measurement.
//!
//! This module owns lightweight timing for the current local CLI/query path. It
//! measures store open, explicit conversation load/context construction, and
//! live model latency without saving benchmark messages to conversation history.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::conversation::{ConversationId, Message, Role};
use crate::gateway::{BifrostGateway, GatewayUrl};
use crate::llm::{BaseUrl, BifrostClient, ModelName};
use crate::store::Store;

const BENCH_PROMPT: &str = "Reply with exactly: ok";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkMode {
    Conversation,
    Local,
    Live,
}

impl BenchmarkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Local => "local",
            Self::Live => "live",
        }
    }

    pub fn may_call_provider(self) -> bool {
        matches!(self, Self::Live)
    }
}

pub struct PerformanceBaseline {
    pub mode: BenchmarkMode,
    pub model: ModelName,
    pub conversation_id: Option<ConversationId>,
    pub store_open: Option<Duration>,
    pub conversation_load: Option<Duration>,
    pub context_build: Option<Duration>,
    pub loaded_messages: Option<usize>,
    pub gateway_ready: Option<Duration>,
    pub first_token: Option<Duration>,
    pub full_response: Option<Duration>,
    pub response_bytes: Option<usize>,
}

pub async fn run(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
) -> Result<PerformanceBaseline> {
    let (store_open, conversation_load, context_build, loaded_messages) = match mode {
        BenchmarkMode::Local => {
            let store_started = Instant::now();
            let _store = Store::open()?;

            (Some(store_started.elapsed()), None, None, None)
        }
        BenchmarkMode::Conversation => {
            let store_started = Instant::now();
            let store = Store::open()?;
            let store_open = store_started.elapsed();
            let conversation_id = conversation_id
                .as_ref()
                .expect("conversation benchmark requires conversation id");

            let load_started = Instant::now();
            let loaded_messages = store.load_messages(conversation_id)?.len();
            let conversation_load = load_started.elapsed();

            let context_started = Instant::now();
            let _ = crate::context::ContextBuilder::build(&store, conversation_id)?;
            let context_build = context_started.elapsed();

            (
                Some(store_open),
                Some(conversation_load),
                Some(context_build),
                Some(loaded_messages),
            )
        }
        BenchmarkMode::Live => (None, None, None, None),
    };

    let (gateway_ready, first_token, full_response, response_bytes) = if mode == BenchmarkMode::Live
    {
        let gateway = BifrostGateway::new(gateway_url);
        let gateway_started = Instant::now();
        gateway.require_running().await?;
        let gateway_ready = Some(gateway_started.elapsed());
        let (first_token, full_response, response_bytes) =
            run_live_request(&base_url, &model).await?;
        (
            gateway_ready,
            first_token,
            Some(full_response),
            Some(response_bytes),
        )
    } else {
        (None, None, None, None)
    };

    Ok(PerformanceBaseline {
        mode,
        model,
        conversation_id,
        store_open,
        conversation_load,
        context_build,
        loaded_messages,
        gateway_ready,
        first_token,
        full_response,
        response_bytes,
    })
}

async fn run_live_request(
    base_url: &BaseUrl,
    model: &ModelName,
) -> Result<(Option<Duration>, Duration, usize)> {
    let llm = BifrostClient::new(base_url.clone(), model.clone());
    let messages = vec![Message {
        id: None,
        parent_message_id: None,
        role: Role::User,
        content: BENCH_PROMPT.to_string(),
        metadata: None,
    }];

    let request_started = Instant::now();
    let mut first_token = None;
    let response = llm
        .stream(&messages, |delta| {
            if first_token.is_none() && !delta.is_empty() {
                first_token = Some(request_started.elapsed());
            }

            Ok(())
        })
        .await?;
    let full_response = request_started.elapsed();

    Ok((first_token, full_response, response.len()))
}

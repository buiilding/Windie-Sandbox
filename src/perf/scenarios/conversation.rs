//! Benchmarks over one existing persisted conversation.

use super::{
    Context, ContextBuilder, ContextParts, CountName, Duration, Instant, MetricName,
    PerformanceBaseline, Result, Store,
};

pub(super) fn record_conversation_benchmark(baseline: &mut PerformanceBaseline) -> Result<()> {
    let store_started = Instant::now();
    let store = Store::open()?;
    let store_open = store_started.elapsed();
    let conversation_id = baseline
        .conversation_id
        .as_ref()
        .expect("conversation benchmark requires conversation id");

    let load_started = Instant::now();
    let active_message_lookup_started = Instant::now();
    let active_message_id = store.active_message_id(conversation_id)?;
    let active_message_lookup = active_message_lookup_started.elapsed();
    let active_path = if let Some(active_message_id) = active_message_id.as_ref() {
        let row_started = Instant::now();
        let messages = store.load_path_to_message_rows(conversation_id, active_message_id)?;
        let row_load = row_started.elapsed();

        let part_started = Instant::now();
        let mut messages = messages;
        store
            .attach_message_parts(&mut messages)
            .context("failed to load active path parts")?;
        let part_load = part_started.elapsed();

        (messages, row_load, part_load)
    } else {
        (Vec::new(), Duration::ZERO, Duration::ZERO)
    };
    let loaded_messages = active_path.0.len();
    let active_path_row_load = active_path.1;
    let active_path_part_load = active_path.2;
    let conversation_load = load_started.elapsed();

    let tree_started = Instant::now();
    let tree_row_started = Instant::now();
    let mut tree = store.load_message_rows(conversation_id)?;
    let tree_row_load = tree_row_started.elapsed();

    let tree_part_started = Instant::now();
    store
        .attach_message_parts(&mut tree)
        .context("failed to load message tree parts")?;
    let tree_part_load = tree_part_started.elapsed();
    let tree_messages = tree.len();
    let tree_load = tree_started.elapsed();

    let tool_schema_started = Instant::now();
    let _ = store.load_tool_schemas(conversation_id)?;
    let tool_schema_load = tool_schema_started.elapsed();

    let context_started = Instant::now();
    let context_active_path_started = Instant::now();
    let context_active_path = store.load_active_path(conversation_id)?;
    let context_active_path_load = context_active_path_started.elapsed();

    let context_system_prompt_started = Instant::now();
    let context_system_prompt = store.system_prompt(conversation_id)?;
    let context_system_prompt_load = context_system_prompt_started.elapsed();

    let context_compaction_started = Instant::now();
    let context_compaction = store.latest_compaction(conversation_id)?;
    let context_compaction_load = context_compaction_started.elapsed();

    let context_flatten_started = Instant::now();
    let _ = ContextBuilder::flatten(ContextParts {
        active_path: context_active_path,
        system_prompt: context_system_prompt,
        compaction: context_compaction,
    });
    let context_flatten = context_flatten_started.elapsed();
    let context_build = context_started.elapsed();

    baseline.record(MetricName::StoreOpen, store_open);
    baseline.record(MetricName::ActivePathLoad, conversation_load);
    baseline.record(MetricName::ActiveMessageLookup, active_message_lookup);
    baseline.record(MetricName::ActivePathRowLoad, active_path_row_load);
    baseline.record(MetricName::ActivePathPartLoad, active_path_part_load);
    baseline.record(MetricName::TreeLoad, tree_load);
    baseline.record(MetricName::TreeRowLoad, tree_row_load);
    baseline.record(MetricName::TreePartLoad, tree_part_load);
    baseline.record(MetricName::ToolSchemaLoad, tool_schema_load);
    baseline.record(MetricName::ContextBuild, context_build);
    baseline.record(MetricName::ContextActivePathLoad, context_active_path_load);
    baseline.record(
        MetricName::ContextSystemPromptLoad,
        context_system_prompt_load,
    );
    baseline.record(MetricName::ContextCompactionLoad, context_compaction_load);
    baseline.record(MetricName::ContextFlatten, context_flatten);
    baseline.count(CountName::ActivePathMessages, loaded_messages);
    baseline.count(CountName::TreeMessages, tree_messages);

    Ok(())
}

//! Benchmark runner entry points.

use super::*;

/// Runs the selected benchmark mode.
pub async fn run(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    _gateway_url: GatewayUrl,
    _base_url: BaseUrl,
    model: ModelName,
    categories: &[BenchmarkCategory],
) -> Result<PerformanceBaseline> {
    let mut baseline = PerformanceBaseline {
        mode,
        model,
        conversation_id,
        scenarios: Vec::new(),
    };

    match mode {
        BenchmarkMode::Conversation => {
            run_conversation_benchmark(&mut baseline, categories)?;
        }
        BenchmarkMode::Local => {
            baseline.scenarios = crate::perf::scenarios::run(categories).await?;
        }
    }

    Ok(baseline)
}

/// Measures the current persisted conversation using its backend-owned head.
fn run_conversation_benchmark(
    baseline: &mut PerformanceBaseline,
    categories: &[BenchmarkCategory],
) -> Result<()> {
    let conversation_id = baseline
        .conversation_id
        .as_ref()
        .expect("conversation benchmark requires conversation id");
    let store_started = Instant::now();
    let store = Store::open()?;
    let store_open = store_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Persistence,
        "storage",
        "store_open",
        "current Windie SQLite store",
        store_open,
    );

    let head_started = Instant::now();
    let head_message_id = selected_session_head(&store, conversation_id)?;
    let head_lookup = head_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Sessions,
        "sessions",
        "selected_head_resolution",
        "backend-owned session head",
        head_lookup,
    );

    if let Some(head_message_id) = head_message_id.as_ref() {
        let row_started = Instant::now();
        let mut path_rows = store.load_path_to_message_rows(conversation_id, head_message_id)?;
        let row_load = row_started.elapsed();
        push_if_selected(
            &mut baseline.scenarios,
            categories,
            BenchmarkCategory::Persistence,
            "storage",
            "message_path_rows",
            "selected session path",
            row_load,
        );

        let part_started = Instant::now();
        store
            .attach_message_parts(&mut path_rows)
            .context("failed to load conversation path parts")?;
        let part_load = part_started.elapsed();
        push_if_selected(
            &mut baseline.scenarios,
            categories,
            BenchmarkCategory::Persistence,
            "storage",
            "message_path_parts",
            "selected session path with text and image parts",
            part_load,
        );
    }

    let tree_row_started = Instant::now();
    let mut tree_rows = store.load_message_rows(conversation_id)?;
    let tree_row_load = tree_row_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Persistence,
        "storage",
        "message_tree_rows",
        "conversation message tree",
        tree_row_load,
    );

    let tree_part_started = Instant::now();
    store
        .attach_message_parts(&mut tree_rows)
        .context("failed to load conversation tree parts")?;
    let tree_part_load = tree_part_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Persistence,
        "storage",
        "message_tree_parts",
        "conversation tree with text and image parts",
        tree_part_load,
    );

    let schema_started = Instant::now();
    let _ = store.load_tool_schemas(conversation_id)?;
    let schema_load = schema_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Persistence,
        "storage",
        "tool_schema_load",
        "conversation tool schemas",
        schema_load,
    );

    let context_path_started = Instant::now();
    let context_path = match head_message_id.as_ref() {
        Some(head_message_id) => store.load_path_to_message(conversation_id, head_message_id)?,
        None => Vec::new(),
    };
    let context_path_load = context_path_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Conversation,
        "context",
        "path_loading",
        "selected session path",
        context_path_load,
    );

    let system_prompt_started = Instant::now();
    let system_prompt = store.system_prompt(conversation_id)?;
    let system_prompt_load = system_prompt_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Conversation,
        "context",
        "system_prompt_loading",
        "conversation-wide system prompt",
        system_prompt_load,
    );

    let compaction_started = Instant::now();
    let compaction = store.latest_compaction(conversation_id)?;
    let compaction_load = compaction_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Conversation,
        "context",
        "compaction_loading",
        "latest conversation checkpoint",
        compaction_load,
    );

    let flatten_started = Instant::now();
    let _ = ContextBuilder::flatten(ContextParts {
        path: context_path,
        system_prompt,
        compaction,
    });
    let flatten = flatten_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Conversation,
        "context",
        "context_flattening",
        "selected path with prompt and compaction",
        flatten,
    );

    let model_context_started = Instant::now();
    let model_context =
        ContextBuilder::build_model_context(&store, conversation_id, head_message_id.as_ref())?;
    let model_context_build = model_context_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Conversation,
        "context",
        "model_context_building",
        "selected session head with schemas",
        model_context_build,
    );

    let serialization_started = Instant::now();
    let _ = crate::llm::benchmark_responses_request_size(
        "openai/test",
        &model_context.messages,
        &model_context.tool_schemas,
    )?;
    let serialization = serialization_started.elapsed();
    push_if_selected(
        &mut baseline.scenarios,
        categories,
        BenchmarkCategory::Serialization,
        "serialization",
        "selected_head_request",
        "provider request from selected model context",
        serialization,
    );

    Ok(())
}

/// Adds one scenario only when its category was selected by the CLI.
fn push_if_selected(
    scenarios: &mut Vec<ScenarioTiming>,
    categories: &[BenchmarkCategory],
    category: BenchmarkCategory,
    layer: &str,
    name: &str,
    fixture: &str,
    duration: Duration,
) {
    if categories.contains(&category) {
        scenarios.push(ScenarioTiming {
            name: name.to_string(),
            category,
            layer: layer.to_string(),
            fixture: fixture.to_string(),
            duration,
        });
    }
}

/// Runs the selected benchmark repeatedly and returns a persistent report.
pub async fn run_report(
    mode: BenchmarkMode,
    conversation_id: Option<ConversationId>,
    gateway_url: GatewayUrl,
    base_url: BaseUrl,
    model: ModelName,
    options: &BenchmarkOptions,
) -> Result<PerformanceReport> {
    let runs = options.runs;
    let mut samples = Vec::with_capacity(runs);

    for _ in 0..runs {
        let baseline = run(
            mode,
            conversation_id.clone(),
            gateway_url.clone(),
            base_url.clone(),
            model.clone(),
            &options.categories,
        )
        .await?;
        samples.push(PerformanceSample::from_baseline(&baseline));
    }

    let summary = PerformanceSummary::from_samples(&samples);

    Ok(PerformanceReport {
        format_version: REPORT_FORMAT_VERSION,
        mode,
        categories: options.categories.clone(),
        model: model.as_str().to_string(),
        conversation_id: conversation_id.map(|id| id.as_str().to_string()),
        runs,
        samples,
        summary,
    })
}

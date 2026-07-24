# Memory-Core: Grounded, Parameter-Efficient Language Model Experiment

**Status:** Draft for implementation  
**Version:** 1.0  
**Decision scope:** First reproducible experiment  
**Primary deliverable:** A comparison report and reproducible code for four 135M systems

## 1. Summary

Memory-Core tests whether useful AI capability can depend less on parameter count by moving factual knowledge out of model weights and into explicit, inspectable memory systems.

The immediate experiment asks whether a small 135M language model can preserve language and reasoning ability while relying less on parametric factual memorization and more on external database interaction. The long-term objective is to determine whether a smaller reasoning core connected to memory, retrieval, tools, and verification can achieve useful capability without endlessly increasing the number of model parameters.

The first experiment will use a standard decoder-only model with two read-only tools over a SQLite database. The model must decide when to retrieve, formulate a useful search, inspect records, resolve stale or conflicting evidence, and abstain when the database does not support an answer.

The experiment compares four systems under matched conditions:

| System | Pretraining | Knowledge access during evaluation |
| --- | --- | --- |
| **A — Standard LM** | Ordinary next-token prediction | Parameters only |
| **A+Tools — Tool control** | Same as A | Explicit `search_memory` and `read_memory` tools |
| **B — LMLM control** | LMLM-style factual externalization | LMLM-compatible factual lookup |
| **C — Memory-Core** | Fact-light, weighted, randomized, reasoning-focused training plus tool trajectories | Explicit `search_memory` and `read_memory` tools |

A+Tools is mandatory. Without it, the experiment cannot distinguish a benefit from Memory-Core’s pretraining method from a benefit caused only by ordinary tool-use post-training.

## 2. Research question and hypotheses

### Immediate research question

Can a language model preserve language and general reasoning while storing fewer arbitrary factual bindings in its parameters and recovering factual competence through explicit database tools?

### Long-term product question

Can an external-memory agent achieve a given level of useful language, reasoning, and knowledge performance with fewer parameters, less parametric factual storage, or lower training cost than a conventional language model trained to memorize the same knowledge?

### Hypotheses

1. **Language retention:** C remains close to A on held-out, low-fact language modeling.
2. **Reasoning retention:** C remains close to or exceeds A on fictional and procedural reasoning.
3. **Reduced factual memorization:** B and C show less unsupported closed-book factual recall than A.
4. **External knowledge recovery:** B and C recover strong factual accuracy when their intended knowledge source is available.
5. **Flexible retrieval:** C performs better than B on multi-step retrieval, query reformulation, stale or conflicting evidence, missing information, and unstructured records.
6. **Pretraining benefit beyond SFT:** C outperforms A+Tools on explicit database tasks.
7. **Immediate updateability:** B and C follow database changes without parameter updates more reliably than A.
8. **Parameter efficiency:** After scaling experiments, a memory-based model can reach a target capability level with fewer parameters or lower training cost than a conventional model, while preserving language and reasoning quality.

The v1 project does not prove the full parameter-efficiency hypothesis because every headline system is fixed at approximately 135M parameters. It establishes the necessary mechanism: the model can externalize factual dependence and use external evidence reliably. A lower closed-book factual score by itself is not success.

## 3. Scope

### In scope for v1

- English-language, decoder-only causal language model.
- A parameter-efficiency research design that separates total parameter count from factual knowledge stored in parameters.
- SmolLM2-style architecture at approximately 135M parameters.
- 512-token context length.
- Existing SmolLM2 tokenizer.
- SQLite database with FTS5 lexical search and BM25 ranking.
- Two read-only model tools: `search_memory` and `read_memory`.
- Synthetic fictional-world data and verifiable tool-use episodes.
- Weighted-token pretraining experiments.
- Standard LM, LMLM, tool-control, and Memory-Core comparisons.
- Supervised tool-use fine-tuning.
- Optional retrieval RL only after the supervised baseline is stable.
- Reproducible evaluation report with fixed held-out databases and questions.

### Explicitly out of scope for v1

- 2.5B or larger model training.
- 128K context length.
- Web search or live external services.
- Learned semantic retrieval, embeddings, vector databases, or a retrieval head.
- Hidden-state or automatic retrieval mechanisms for Memory-Core.
- Writable memory in the first comparison.
- Episodic/procedural memory implementation.
- Production-grade lifelong learning, safety, or autonomous operation.
- Claims that the model has human-like memory or intelligence.

## 4. Product definition

The v1 product is a research harness, not a general assistant. It consists of:

1. A trainable 135M causal language model.
2. A deterministic miniature database environment generated per episode.
3. A serialized tool-call protocol understood by the model.
4. Training pipelines for pretraining, tool SFT, and later optional RL.
5. An evaluation suite that measures both answer quality and evidence use.
6. A report that makes all comparisons and transformations inspectable.

The v1 product exists to test a parameter-efficient architecture hypothesis, not merely to create a retrieval demo. The intended division of labor is:

- **Core model parameters:** language, abstraction, reasoning, planning, generalization, and communication.
- **External memory:** factual knowledge, documents, changing information, and accumulated experience.
- **Tools:** retrieval, verification, transformation, and other inspectable operations.

The conceptual long-term system has internal semantic, episodic, and procedural memory plus external information sources. In v1, only the external read-only database is implemented. “Internal memory” means model parameters for this experiment; durable writable internal memory is a later product stage, not a hidden part of the first experiment.

## 5. Fixed experimental configuration

These values are frozen for the first comparison unless a run is explicitly labeled a configuration experiment.

| Setting | v1 choice |
| --- | --- |
| Architecture | SmolLM2-style decoder-only transformer |
| Size | Approximately 135M parameters |
| Tokenizer | Existing SmolLM2 tokenizer |
| Language | English |
| Context | 512 tokens |
| Smoke-test budget | 10M tokens |
| Comparison budget | 100M–300M tokens, selected before final runs |
| Database | SQLite + FTS5 |
| Retrieval | Lexical matching with BM25; deterministic ranking |
| Training | LitGPT or a minimal compatible training harness |
| Tool post-training | TRL or a minimal compatible SFT/RL harness |
| Initial hardware | RTX 5070 Ti with 16GB VRAM |

All four systems must use the same architecture, tokenizer, context length, token budget, optimizer, learning-rate schedule, batch-size policy, training-step budget, evaluation data, and declared random seeds. If a framework limitation prevents exact matching, the limitation must be recorded in the report.

## 6. System behavior

For each user question, the model may:

1. Answer directly when retrieval is unnecessary.
2. Search the database when external evidence is needed.
3. Read one or more candidate records.
4. Reformulate a failed or overly broad query.
5. Compare records using recency, provenance, and confidence.
6. Derive an answer from multiple records.
7. Abstain when the required evidence is absent or insufficient.

The database returns observations. It must not generate a final answer for the model. The model is responsible for deciding what the evidence means and whether it supports the response.

### Initial tool contracts

```json
{
  "name": "search_memory",
  "description": "Search the external knowledge database.",
  "arguments": {
    "query": "string",
    "collection": "string | null",
    "before": "string | null",
    "after": "string | null",
    "limit": "integer"
  }
}
```

```json
{
  "name": "read_memory",
  "description": "Read a database record by ID.",
  "arguments": {
    "record_id": "string",
    "offset": "integer",
    "limit": "integer"
  }
}
```

Tool behavior must be deterministic for a fixed database, query, and seed. Invalid arguments must return structured errors rather than crashing the episode.

### Record shape

The database record must contain at least:

```json
{
  "id": "doc_0182",
  "collection": "project_documents",
  "title": "Revised Mercury schedule",
  "content": "The launch was moved from September 10 to September 12.",
  "source": "schedule-revision.md",
  "created_at": "2026-07-18",
  "valid_from": "2026-07-18",
  "confidence": 0.98
}
```

Search results return only record IDs, titles, metadata, and bounded snippets. Full content is available only through `read_memory`.

## 7. Data design

### 7.1 Master corpus and derived views

Create one master corpus and derive matched training views. This prevents unrelated source data from confounding the comparison.

- **A standard view:** original text, all non-padding tokens weighted 1.0.
- **B LMLM view:** the same source material transformed using the selected LMLM reproduction procedure, with factual spans externalized according to that method.
- **C Memory-Core view:** fact-light selection, factual-span weighting, typed replacement where appropriate, fact-binding randomization, and added reasoning/tool data.

The total token budget, not necessarily the byte-for-byte content, must be matched. Every transformation must produce an auditable manifest containing source ID, output ID, token count, transformation type, and random seed.

### 7.2 Starting Memory-Core mixture

Use these as initial shares for C; they are tunable experiment parameters, not product truths:

| Data class | Starting share |
| --- | ---: |
| Low-fact language: fiction, dialogue, essays | 35% |
| Math, code, instructions, procedural text | 20% |
| Fictional-world reasoning | 20% |
| Randomized factual bindings | 10% |
| Explicit database-tool episodes | 15% |

Prioritize language, explanations, arguments, procedures, conceptual science, math, code, descriptions, and transformations. Reduce repetitive biographies, trivia lists, mutable specifications, reference tables, entity-heavy SEO pages, and fact-dense pages. Do not remove concepts or general world understanding merely because a document contains facts.

### 7.3 Factual-span weighting

Each training example supports:

```json
{
  "input_ids": [1, 2, 3],
  "labels": [2, 3, 4],
  "loss_weights": [1.0, 0.05, 1.0]
}
```

The implementation computes unreduced token cross-entropy and applies the weights:

```text
token_losses = cross_entropy(logits, labels, reduction="none")
loss = sum(token_losses * loss_weights) / max(sum(loss_weights), 1)
```

Initial branches must test factual-value weights of **0.00, 0.05, and 0.10**. Normal language, procedural/reasoning tokens, randomized fictional facts, and tool-call tokens receive weight 1.0. Tool observations and user/system text in serialized trajectories receive weight 0.0 unless a specific control experiment says otherwise. Padding tokens are excluded.

Weighting alone is not assumed to prevent memorization. It must be combined with corpus selection, randomization, typed placeholders, and explicit tool episodes.

### 7.4 Synthetic fictional worlds

Generate worlds with changing names, values, rules, ordering, wording, document order, and distractors. Required task families:

- graph traversal;
- temporal ordering;
- causal chains;
- constraint satisfaction;
- scheduling;
- arithmetic;
- program execution;
- planning;
- multi-hop deduction;
- counterfactual worlds.

Randomized facts receive full loss because the binding changes between episodes; the useful behavior is copying, manipulating, and composing supplied values.

### 7.5 Tool episode generator

The generator is a first-class component and must create, for each episode:

- a fresh miniature database;
- a user question;
- a known correct answer or abstention label;
- a valid action trajectory or set of valid trajectories;
- relevant records;
- distractors;
- optional stale, duplicate, contradictory, or missing records;
- a machine-checkable evidence set;
- a deterministic episode seed.

Required episode families:

| Family | Required behavior |
| --- | --- |
| Retrieval required | Search and read before answering |
| Retrieval unnecessary | Answer directly without a tool |
| Failed first query | Reformulate and retry |
| Too many matches | Narrow the query or inspect candidates |
| Buried evidence | Evaluate more than the first result |
| Stale/current records | Prefer current evidence |
| Contradiction | Compare provenance and confidence |
| Missing information | Abstain clearly |
| Multi-hop | Retrieve, derive, and issue a second query |
| Counterfactual | Follow the generated database over prior associations |

The initial episode target is 10,000 generated episodes, with at least 30% retrieval-unnecessary, 20% failed-first-query, 15% stale/conflicting evidence, and 10% answer-absent cases. These percentages are minimum coverage targets, not claims about the optimal mixture.

## 8. Training plan

### Phase 0 — Infrastructure smoke test

Train an unmodified A baseline on 10M tokens. Verify loss descent, checkpoint save/load, deterministic evaluation, coherent generation for the model size, and functioning metric scripts.

**Exit gate:** the baseline run is reproducible and the data/metrics pipeline produces a complete report.

### Phase 1 — Database environment

Implement SQLite schema, FTS5 indexing, BM25 ranking, filters, snippets, pagination, record reads, structured errors, deterministic seeding, and unit tests. Build the environment before model-specific tool training.

**Exit gate:** fixed database tests prove ranking, filtering, pagination, stale-record fixtures, contradictory-record fixtures, missing-record behavior, and read offsets.

### Phase 2 — Episode and evaluation generators

Implement the synthetic world generator, tool trajectory generator, answer/evidence oracle, held-out split, and fixed evaluation database. Ensure names, facts, and layouts do not leak from train to evaluation.

**Exit gate:** at least 10,000 training episodes and a fixed held-out suite can be regenerated from recorded seeds, with no unverifiable labels.

### Phase 3 — LMLM control

Reproduce B with the same architecture, source-token budget, and shared evaluation facts. Record exactly which portions of the original method are implemented and any deviations.

**Exit gate:** B runs end-to-end and its knowledge access path is measurable separately from explicit tool use.

### Phase 4 — Memory-Core pretraining ablations

Train sequential C checkpoints with matched budgets:

| Checkpoint | Added technique |
| --- | --- |
| C0 | Fact-light corpus |
| C1 | C0 + factual-span weighting |
| C2 | C1 + factual-binding randomization |
| C3 | C2 + fictional-world reasoning |
| C4 | C3 + counterfactual and conflict training |
| C5 | C4 + explicit database-tool trajectories |

Run the three factual-loss weights across the smallest useful subset first. Expand only when the smoke results justify the compute.

**Exit gate:** every checkpoint has its config, seed, token count, checkpoint path, data manifest, and evaluation report.

### Phase 5 — Shared tool-use SFT

Apply identical explicit-tool SFT data to A and C5, producing A+Tools and C. Include positive and negative retrieval decisions. Train syntax and behavior together, but evaluate them separately.

The SFT data must cover retrieval necessity, query formulation, failed queries, broad results, candidate selection, stale/conflicting records, multi-hop retrieval, unsupported evidence, and abstention.

**Exit gate:** the models produce valid tool calls reliably, avoid tools on direct-answer cases, and do not fabricate tool observations in held-out episodes.

### Phase 6 — Optional retrieval RL

Run only after SFT is stable and the four-system baseline is complete. Use the generated environment as the rollout state and an automatic reward function. RL is an optimization phase, not a prerequisite for validating the initial hypothesis.

Suggested reward structure:

```text
R = 2 * answer_correct
  + retrieval_decision_correct
  + evidence_relevant
  + answer_grounded
  + 0.5 * query_reformulated_successfully
  - 0.1 * number_of_calls
  - 2 * fabricated_observation
```

The report must show reward components independently so a high score cannot hide poor grounding or excessive calls.

### Phase 7 — Scale or stop

Scale to 360M only if the 135M comparison meets the success criteria and the result is reproducible. Do not add 128K context, web search, or writable memory before the core result is understood.

## 9. Evaluation

Every report must include A, A+Tools, B, and C, plus confidence intervals or repeated-seed variation where compute allows.

| Area | Metric | Question |
| --- | --- | --- |
| Language | Low-fact held-out perplexity | Was ordinary language ability retained? |
| Reasoning | Fictional/procedural accuracy | Can the model reason without real-world recall? |
| Parametric leakage | Closed-book factual recall and unsupported-claim rate | How much factual dependence remains in parameters? |
| Open-book accuracy | Correct answers with intended knowledge access | Can external knowledge restore performance? |
| Retrieval decision | Precision, recall, and F1 | Does the model search only when useful? |
| Search quality | Success@N | Does the query surface required evidence? |
| Record choice | Record-selection accuracy | Does it read the right evidence? |
| Reformulation | First-failure recovery rate | Can it change strategy after a failed search? |
| Grounding | Evidence-supported answer rate | Is the final answer supported by read records? |
| Update obedience | Accuracy after database replacement | Does behavior change without retraining? |
| Conflict handling | Authority/recency resolution accuracy | Does it handle contradictory sources? |
| Abstention | No-result abstention rate | Does it avoid unsupported answers? |
| Efficiency | Calls and records read per solved task | How costly is retrieval? |

### Evaluation controls

- Closed-book tests disable all databases and tools.
- Open-book tests use facts absent from held-out training data.
- Counterfactual tests intentionally contradict familiar real-world associations.
- Update tests change a record while keeping the question constant.
- Missing-information tests remove the answer entirely.
- The same database facts and questions are used for all eligible systems.
- Metrics distinguish invalid syntax, bad search, bad record selection, unsupported reasoning, and incorrect final answers.

### Parameter-efficiency evaluation (post-v1)

The v1 report must not claim that Memory-Core uses fewer total parameters; it only compares systems at the same parameter count. Once the 135M mechanism is validated, run a scale comparison using the best Memory-Core configuration and a conventional baseline at multiple sizes, beginning with 135M and 360M.

Report, for each model size:

- total and trainable parameters;
- training tokens, training compute, and wall-clock cost;
- low-fact language quality;
- fictional/procedural reasoning quality;
- closed-book factual recall and unsupported-claim rate;
- open-database accuracy and grounded-answer rate;
- retrieval efficiency;
- capability per parameter and capability per unit of training compute.

The parameter-efficiency claim is supported only if Memory-Core reaches a pre-registered target capability with fewer parameters or lower training cost, without unacceptable degradation in language, reasoning, grounding, or update obedience.

## 10. Success criteria and first milestone

The first milestone is a reproducible mechanism report, not yet a proof of lower total parameter count. It must contain:

- exact configs and seeds;
- source-corpus and transformation manifests;
- all A/A+Tools/B/C checkpoints;
- held-out databases and questions;
- training and evaluation scripts;
- closed-book and open-book results;
- tool-use and grounding results;
- database-update and abstention tests;
- ablation results;
- known deviations from the intended design.

Before scaling, the 135M model should demonstrate all of the following:

- valid tool-call syntax above 95% on held-out tool prompts;
- retrieval-decision accuracy above 80%;
- higher open-database accuracy than its closed-book accuracy on retrieval-required tasks;
- lower unsupported factual recall than A;
- fictional-reasoning performance comparable to A;
- immediate obedience to changed database facts;
- reliable abstention when the answer is absent;
- no material collapse in low-fact language quality;
- C outperforming A+Tools on at least the core grounded-retrieval composite, not merely on raw answer accuracy.

The final criterion must be reported with its component metrics. A model should not pass by making fewer claims or more tool calls while failing to ground answers.

This milestone is a prerequisite for the parameter-efficiency study. It demonstrates that external memory can replace some parametric factual storage; it does not by itself demonstrate that a smaller model outperforms or matches a larger conventional model.

## 11. Implementation requirements

### Required components

```text
memory-core/
├── data/
│   ├── language/
│   ├── fictional_worlds/
│   ├── randomized_bindings/
│   └── tool_episodes/
├── memory_env/
│   ├── database.py
│   ├── search.py
│   └── environment.py
├── training/
│   ├── weighted_loss.py
│   ├── pretrain.py
│   └── sft.py
├── eval/
│   ├── factual_leakage.py
│   ├── retrieval_decision.py
│   ├── search_quality.py
│   ├── grounding.py
│   └── update_obedience.py
├── configs/
│   ├── standard_135m.yaml
│   ├── lmlm_135m.yaml
│   ├── standard_tools_135m.yaml
│   └── memory_core_135m.yaml
└── reports/
```

### Reproducibility requirements

- Record code version, configuration, seed, hardware, dependency versions, token counts, and wall-clock duration for every run.
- Save dataset manifests and generator seeds.
- Keep training, validation, and evaluation databases disjoint.
- Version tool schemas and serialized trajectory formats.
- Make database results deterministic for fixed inputs.
- Preserve intermediate checkpoints C0–C5.
- Fail loudly on malformed labels, invalid loss weights, missing evidence labels, and tool protocol violations.

## 12. Risks and mitigations

| Risk | Mitigation |
| --- | --- |
| The model learns not to answer rather than becoming grounded | Track answer rate, abstention precision, unsupported-claim rate, and calibrated reward components. |
| Synthetic data teaches a narrow benchmark pattern | Randomize entities, wording, layouts, distractors, and episode structure; hold out generators and templates where possible. |
| Factual masking leaks information through context | Combine weighting with fact-light selection, randomization, typed replacement, and leakage tests. |
| A+Tools gets more or better training than C | Use identical tool SFT data, budgets, and evaluation procedures. |
| BM25 makes the task mostly keyword matching | Keep lexical retrieval for v1 to make the causal experiment inspectable; add semantic retrieval only as a later comparison. |
| LMLM reproduction is not faithful | Document the reproduction boundary and treat B as a control with explicit limitations. |
| 135M capacity is too small for reliable tool behavior | Treat failure as a result; first verify syntax, data, and environment independently before scaling. |
| Hardware limits make all branches expensive | Use 10M-token smoke tests, staged ablations, fixed evaluation subsets, and scale only after gates pass. |

## 13. Parameter-efficiency roadmap

After the 135M comparison succeeds, run the following controlled scale study:

1. Select the best Memory-Core checkpoint and the matched conventional baseline.
2. Train or evaluate both approaches at 135M and 360M under recorded, comparable budgets.
3. Add a larger conventional reference only if the hardware and budget permit a fair run.
4. Compare capability at fixed parameter count, fixed compute, and fixed target quality.
5. Pre-register the acceptable quality trade-offs before inspecting the scale results.

The goal is not to minimize parameters at any cost. The goal is to test whether externalized knowledge lets the reasoning core stay smaller while maintaining useful capability, reliable grounding, and immediate knowledge updates.

## 14. Deferred memory roadmap

Writable memory begins only after read-only retrieval is reliable and the first comparison is complete.

### Stage 1: Semantic memory

Add `write_memory`, `update_memory`, and `delete_memory`. A write must include content, memory type, source, timestamp, confidence, and retention reason. Train and evaluate:

```text
experience → identify reusable information → verify → deduplicate
→ write → retrieve later → update or delete when contradicted
```

### Stage 2: Episodic memory

Store event- or interaction-specific records with time, participants, outcome, and provenance. Evaluate temporal recall, relevance, and privacy boundaries.

### Stage 3: Procedural memory

Store reusable workflows only after semantic writes can be safely validated and revised. Evaluate execution success, versioning, and behavior when a procedure becomes stale.

These stages are not part of the v1 success claim.

## 15. Immediate execution backlog

1. Create the project layout and configuration schema.
2. Implement weighted-token data validation and loss tests.
3. Implement SQLite/FTS5 records, search, read, filters, snippets, and pagination.
4. Add deterministic fictional-world and tool-episode generators.
5. Train and report the 10M-token A smoke test.
6. Generate the initial 10,000 training episodes and fixed held-out suite.
7. Implement the B reproduction boundary and configuration.
8. Train C0–C5 with the declared loss-weight branches.
9. Train A+Tools and C with identical tool SFT data.
10. Run the four-system evaluation and publish the comparison report.
11. Decide whether to run retrieval RL.
12. Decide whether the evidence justifies scaling to 360M.

## 16. Open decisions before final runs

These items should be resolved in the experiment configuration and recorded before comparison runs begin:

- The exact total token budget within the 100M–300M range.
- The exact SmolLM2 checkpoint/configuration and whether initialization is from scratch or continued training.
- The exact LMLM paper/procedure and the reproducibility boundary for B.
- The factual-span annotation method and quality sample size.
- The train/validation/test split policy for synthetic generators and database templates.
- The maximum episode length and tool-call budget.
- The answer format and abstention phrase/schema used by evaluation.
- The minimum number of random seeds for the headline result.
- The composite grounded-retrieval score and its pre-registered component weights.

No new model size, memory type, or retrieval technology should be introduced into the headline comparison after these decisions are frozen.

# Skills

## Purpose

<!-- Explain skills as on-demand instructions supplied by an installed plugin. -->

## Owns

<!-- Describe declaration, indexing, loading, and model-context inclusion. -->

## Does not own

<!-- Distinguish skill instructions from executable tools and user prompts. -->

## Main flow

1. <!-- Include compact skill metadata in the plugin index. -->
2. <!-- Let the model request one skill when relevant. -->
3. <!-- Load its instructions into the current model request. -->

## Important invariants

- <!-- Skill text remains package-owned runtime metadata. -->
- <!-- Reading a skill does not persist it as user-authored conversation content. -->

## Related code

- <!-- Plugin skill manifest and loader modules. -->
- <!-- `src/tool/builtin.rs` -->
- <!-- `src/runtime/context.rs` -->

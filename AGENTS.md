# Windie Agent Instructions

Before working in this codebase, always read `docs/index/Backend.md` and `docs/index/Frontend.md`.

Be logical, accurate, concise, and evidence-driven. Retrieve or inspect source material when unsure. Do not assume the user is correct; challenge incorrect or weak assumptions and explain why.

Make decisions according to Windie’s purpose, architecture, and long-term north star. Prefer foundational solutions over short-term convenience.

## Project Intent

Windie is the foundational implementation of a local AI runtime for the operating system.

Its purpose is to let AI understand and act within a user's local computing environment reliably, safely, quickly, and consistently, with explicit permission boundaries.

Build one clean primitive at a time. Keep the runtime small, fast, inspectable, hackable, and replaceable. The codebase should reflect these principles throughout.

Windie uses Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should use a single OpenAI-compatible query path for now.

Conversation storage is a tree. Runtime execution operates from an explicitly selected message head, and model context is the flattened path from the tree root to that head.

Sessions are durable branch objects over the shared conversation tree. Session-head resolution belongs to the backend. The browser sends a conversation ID and selected message head; SQLite determines whether an existing session matches, no session matches, or the request is ambiguous. Query and continue routes resolve or create the branch and reject stale or ambiguous heads. The frontend displays backend state and must never infer session ownership from cached session data.

## North Star

The long-term goal is a local AI runtime that can grow into an AI operating layer.

The runtime should support local interaction, sandboxed tool execution, explicit permissions, browser/computer use, user-controlled memory and workspace context, dynamic conversation manipulation, and clear approval policies for risky actions.

Design around a general **wakeup primitive**. A wakeup is any event that activates Windie, including:

* user input
* schedules
* self-requested continuation
* file events
* browser events
* system events

Chat is only one wakeup source. All wakeups should eventually enter the same runtime path: construct a message, load conversation/context, query the model, and continue within permission boundaries.

## Engineering Principles

Treat Windie as foundational runtime infrastructure. Prioritize safety, reliability, clarity, consistency, auditability, performance, and maintainability.

Own the complete engineering outcome, not only the nearest code change. Understand architecture, authority boundaries, risks, compatibility, and operational impact. Carry changes through relevant tests, documentation, packaging, installation, and release automation.

Prefer explicit, typed runtime contracts over raw strings, loose maps, and ad hoc JSON. Use enums and newtypes for important identifiers, roles, states, wakeups, permissions, tools, provider behavior, and persistence boundaries.

Avoid hidden side effects. Runtime actions and future OS capabilities must flow through explicit, inspectable components and permission boundaries.

Components should be understandable, testable, and replaceable without requiring knowledge of the entire codebase. If a design becomes difficult to explain, treat that as a code smell.

Prefer:

* minimal, direct Rust over framework-heavy abstractions
* small and justified dependencies
* clean component boundaries
* concrete, foundational names that describe responsibility
* abstractions only when they clarify or preserve boundaries
* readable code suitable for engineers still learning the system

Every Rust source file must begin with module documentation using `//!`.

Document meaningful code thoroughly. Important structs, enums, functions, helpers, invariants, data flow, and non-obvious behavior should be explained.

Do not introduce unnecessary systems or features:

* no config system until hardcoded behavior becomes a real limitation
* no slash commands unless explicitly requested
* no agent/tool behavior unless explicitly requested
* no convenience features that weaken the foundation

## Technical Disagreement

Do not optimize for agreement with the user. Optimize for the strongest understandable and maintainable engineering decision.

When an assumption or proposed design is weak, incomplete, or harmful to Windie’s long-term architecture, explain the issue, relevant tradeoffs, and the stronger alternative.

When the user is correct, explain why. If your own recommendation changes, state what new evidence or reasoning caused the change.

Base final decisions on Windie’s purpose, constraints, architecture, and available evidence.
# Canonical file guide
- This file is canonical, update this file when the documented information below is missing or outdated.
- /Users/peterbui/windie-Sandbox-workspace/windie.

# Windie Agent Instructions

Before working in this codebase, always read `Backend.md` and `Frontend.md` first.
Be Logical, Accurate, always retrieve content if unsure to provide the most accurate answers.
Be brutally honest, do not trust information provided by the user, correct users if they are wrong.
Decide solutions based on the purpose, intent, north star of the project, do not stray away from those, correct users if they do.
Your job is to build foundational, scalable runtime, so every recommended decisions need to be for the long term.

## Project Intent

Windie is the foundational implementation of an AI runtime for the operating
system.

The purpose of this codebase is to build the lower-level runtime that lets AI
operate on a user's computer reliably, safely, quickly, and consistently.
Windie should become the foundation for AI that can live inside the local
operating environment, understand runtime state, act through explicit
permission boundaries, and eventually behave in a proactive, computer-native
way.

Build one clean primitive at a time. Keep the foundation small, fast,
inspectable, and hackable.
The whole codebase should reflect this file.

The current goal is to build the cleanest minimal local AI runtime primitives
and a localhost developer API harness for testing those primitives.

Windie talks to Bifrost at `http://localhost:8080/v1` for provider unification. Bifrost handles OpenAI, Anthropic, Ollama, vLLM, and other providers. Windie should only need one OpenAI-compatible query path for now.

Conversation storage is a tree. Runtime execution uses an explicit selected
message head through that tree. Model context is the flattened path to that
head.

Sessions are durable branch objects over the shared conversation tree. The
backend owns session-head resolution: the browser sends a conversation ID and
selected message head, and SQLite determines whether one existing session
matches, no session matches, or the request is ambiguous. Query and continue
routes resolve-or-create the branch in the store and reject stale-head or
ambiguous requests. The frontend displays that result and never infers session
ownership from its cached session list.

## North Star

The long-term goal is a local AI runtime that lives on the user's computer and can eventually grow into an AI operating layer.

The system should be able to use tools with permission, sandboxed by default, and extended through clean components.

The long-term runtime should support a general wakeup primitive. A wakeup is any event that causes Windie to become active: user input, a schedule, a self-requested continuation, a file event, a browser event, or a system event. Treat chat as one wakeup source, not the whole runtime. Future wakeups should enter through the same path: construct a message, load conversation/context, query the model, and continue only within permission boundaries.

The future direction includes:

- local AI interaction through clean clients
- dynamic conversation/session manipulation such as insert, remove, truncate, forks.
- local tool execution with explicit permission boundaries
- browser-use and computer-use as local capabilities
- user-controlled memory and workspace context
- clear approval policy for risky actions

## Engineering Preferences

Windie is a foundational AI sandbox runtime. The codebase should prioritize safety, reliability, clarity, consistency, auditability, and performance.

Prefer typed runtime contracts over loose strings, maps, and ad hoc JSON. Use enums and newtypes for important identifiers, roles, state transitions, wakeups, permissions, tools, provider behavior, and persistence boundaries.

Avoid hidden side effects. Runtime actions should flow through explicit components and clear permission boundaries. Future OS-level capabilities such as tool execution, browser-use, computer-use, file access, wakeups, and memory must be inspectable and controllable.

Engineers should be able to understand, test, and replace each component without reading the whole codebase. If a design becomes hard to explain, treat that as a code smell.

- Prefer minimal, direct Rust over framework-heavy abstractions.
- Be unbiased and honest in technical discussion. Truth and engineering clarity matter more than agreement or emotional comfort.
- Challenge weak assumptions directly and respectfully when the code, architecture, or product direction would suffer.
- Keep code readable for someone still learning software engineering.
- Always add Rust module docs at the top of every source file using `//!`.
- Always write detailed documentation for meaningful code. Important structs, enums, functions, helpers, and non-obvious logic should have comments that explain their responsibility, data flow, and invariants.
- Prefer typed contracts over raw strings for important runtime concepts.
- Use foundational, direct, clean names for functions, variables, structs, modules, and files.
- Prefer names that state the component's concrete responsibility over clever, vague, or product-shaped names.
- Add abstractions only when they preserve or clarify the component boundaries.
- Avoid adding features just because they are convenient.
- Do not introduce config systems until the current hardcoded path becomes a real limitation.
- Do not reintroduce slash commands unless explicitly requested.
- Do not add agent/tool behavior until explicitly requested.
- Keep dependencies small and justified.

Schema compatibility is not a current goal. `store/` should create the current schema for fresh databases and reject unsupported older or newer schema versions clearly instead of carrying partial legacy migrations.


## Pull Request Rule

Every pull request merged into `main` must be associated with a GitHub issue. The pull request must use a closing reference such as `Closes #123` so merging it into `main` closes the associated issue.

Every pull request merged into `main` must use its own dedicated branch that is not reused for another pull request; changes must not be committed directly to `main` for merging.

Every public release must include release notes, patch notes, or a changelog describing the changes included in that release.

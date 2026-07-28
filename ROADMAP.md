# Roadmap

Windie is being built as a foundational, transparent, fast, general AI harness
that lives on the operating system and gives other software a clean runtime to
build on top of.

This roadmap describes direction and sequencing, not promised dates. Some items
are product work, while others are foundational audits required to make the
runtime trustworthy before adding more autonomy.

## Next — finish the everyday harness experience

These are the next user-visible capabilities needed to make the current curl-
installed Windie runtime understandable and useful without project history.

- Improve the installed-runtime experience: make it clear how users open the
  Inspector, see that the Windie API is running, and stop the API when they are
  finished. A tray application is one option under consideration, but the
  final approach depends on packaging and notarization.
- Generate useful titles for conversations.
- Generate useful titles for sessions so branches are easier to distinguish.
- Render images in the transcript alongside streamed text, tool calls, and
  other assistant output.
- Make tool outputs easier to scan and understand in the Inspector.
- Add a first set of curated plugins and skills.
- Let users install and add MCP providers, plugins, and skills through the
  product instead of requiring every provider to be code-owned by Windie.
- Add curated MCP providers for browser-use and Minecraft.
- Export conversation context in a portable, copyable form that can be moved
  into another harness, model, or web AI when users change models, run out of
  Windie credits, or want to continue elsewhere.

## In parallel — solidify the foundation

This work is less visible, but it determines whether Windie can safely become
more autonomous.

- Expand the benchmark workflow around the runtime’s current capabilities and
  track regressions with meaningful baselines.
- Expand the testing workflow with edge cases, invariants, and human-guided
  scenarios that automated tests cannot fully define by themselves.
- Audit MCP session management so extension processes start when needed, stop
  when no longer needed, recover correctly, and do not waste the user’s CPU or
  memory.
- Audit context-manipulation operations such as branching, editing, deleting,
  truncating, and forking. Define when each operation is safe, what it should
  do to sessions, and when it must be rejected.
- Keep permissions, tool execution, session state, context, and outcomes
  explicit and inspectable as new capabilities are added.

## Later — make Windie proactive

Windie should eventually do more than respond while the user is typing.

- Add periodic wakeups: scheduled or self-determined triggers that re-query an
  idle session with a system-generated prompt so the AI can continue acting
  within its permissions when the user is away.
- Support other wakeup sources through the same runtime path, including user
  input, schedules, file events, browser events, system events, and
  model-requested continuation.
- Explore ways for Windie to use existing Codex or Claude subscriptions rather
  than requiring a separate API key. This depends on what those subscription
  products expose; Kimi Code is the current example of the desired model.
- Enable agents to communicate with and coordinate with other agents.
- Allow agents to propose or perform extension installation within explicit
  permission boundaries.

## Long-term direction — an AI operating layer

The long-term goal is a sentient, proactive AI that lives on the computer while
remaining transparent, inspectable, and controllable. Chat is only one wakeup
source; the runtime should be able to observe permitted events, decide when to
act, and record what happened.

- Move from a harness running beside the user’s operating system toward a
  sandboxed operating system designed for AI-native work.
- Let humans remotely control that environment through familiar interaction
  such as clicking and typing, preserving a human-compatible path while
  AI-native interfaces mature.
- Keep the harness, permissions, context, tools, wakeups, and agent activity
  visible rather than hiding them behind an opaque platform.

## Intentionally deferred

These are deliberately not the immediate focus. The harness needs to become
reliable before they are added.

- The sandboxed operating system.
- Remote control of that operating system.
- Voice interaction. Typing is currently the more deterministic interface; voice
  comes after the core harness is solid.

The concrete work for each roadmap item should become a GitHub Issue only when
the scope and acceptance criteria are clear.

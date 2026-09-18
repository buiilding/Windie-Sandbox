# Windie official UI design

Windie is evolving from a chat-only interface into a user-facing workspace. The official UI should feel familiar to users of applications such as ChatGPT while presenting Windie's unique runtime features in a coherent way.

## Product concepts

The official UI must accommodate these connected concepts:

- **Conversations** — durable user-facing workspaces containing the conversation history.
- **Conversation graphs** — the branching structure of a conversation, allowing users to navigate nodes and branch, fork, or delete conversation paths.
- **Sessions** — runtime execution instances associated with a conversation and a selected graph head.
- **Wakeups** — events that activate or resume Windie, including scheduled, idle, user-requested, file, browser, or system events.
- **Computers** — environments or virtual machines available for Windie to operate.
- **Computer controls** — the remote-control experience and actions used to interact with a computer through mouse and keyboard input.
- **Talents/extensions** — installed capabilities that a session can use.

These concepts should feel connected without being collapsed into one undifferentiated chat interface.

## Three-layer workspace model

The official UI has three layers with distinct responsibilities.

### Left sidebar: global navigation

The left sidebar contains:

- New chat
- Wakeups
- Computers
- Talents/extensions
- Recent conversations

Selecting an item changes the state of the central workspace. Recent conversations open their transcript. Wakeups, Computers, and Talents open their respective user-facing management or workspace views. New chat opens a fresh conversation workspace.

The left sidebar answers: **Where in Windie am I?**

### Center: primary workspace

The center is the main surface where the user works.

For a conversation, it contains the transcript and composer. When the user selects another area from the left sidebar, the center can display that area's primary view instead.

The composer is specifically for composing user input. It should not be responsible for presenting session management or wakeup configuration.

The center answers: **What am I primarily working on?**

### Right sidebar: contextual dock

The right sidebar is a resizable panel that opens beside the central workspace. It augments the current work without taking over or replacing the transcript.

Possible contextual tools include:

- **Computer** — opens the virtual-machine view so the user can remotely control it with a mouse and keyboard.
- **Graphs** — opens the graph for the current conversation. Users can navigate to different nodes and perform graph actions such as branching, forking, or deleting.
- Future contextual tools such as review, files, browser, activity, or side chat.

The right sidebar answers: **What supporting view or tool do I want beside my current work?**

## Important UX boundaries

- Conversations are the primary user-facing workspaces.
- Conversation graphs are a contextual view of the current conversation, not an unattractive replacement for the transcript.
- Sessions represent runtime state and execution. They should not be embedded in the composer.
- Wakeups are first-class activation sources, not merely a session control hidden inside the composer.
- Computers are environments or resources; computer controls are the actions and remote-control surface used against them.
- Talents/extensions are capabilities and need a clear discovery and management experience.
- The transcript remains stable and visible while contextual tools such as the graph or computer view are open beside it.
- The left sidebar changes the primary workspace; the right sidebar augments the current workspace.

## Relationship to the Inspector

The existing Windie Inspector was developed incrementally as features appeared. It contains much of the working runtime behavior, but its information architecture reflects that history:

- sessions and wakeups are exposed through the composer area;
- conversation graphs are presented as a separate utility view rather than a polished contextual workspace;
- tools and extensions are mixed into inspector and settings surfaces; and
- Computers and computer controls do not yet have a coherent user-facing home.

The official UI should therefore be designed as a new presentation layer rather than copied directly from the Inspector. Inspector's existing API, conversation state, session runtime, streaming, tools, providers, extensions, approvals, and wakeup behavior can eventually be connected underneath this clearer design.

## Core mental model

> The left sidebar is where the user is in Windie. The center is what they are primarily working on. The right sidebar provides supporting tools and views beside that work.

The goal is a smooth workspace experience in which users can move between a conversation, its graph, the session executing it, wakeups, computers, computer controls, and talents without losing the context of the work in the transcript.

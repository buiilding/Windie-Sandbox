import { useEffect, useMemo, useRef, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import MessageRow, { PendingAssistantRow } from "@/components/windie/MessageRow";
import Composer from "@/components/windie/Composer";
import ToolApprovalPrompt from "@/components/windie/ToolApprovalPrompt";
import { executionToolCount, isExecutionNode } from "@/lib/treeProjection";
import { ChevronDown, ChevronUp, MoreHorizontal } from "lucide-react";

function transcriptItems(nodes) {
  const items = [];
  let executionNodes = [];

  const flushExecution = () => {
    if (!executionNodes.length) return;
    items.push({
      type: "execution",
      id: `transcript-execution:${executionNodes[0].node.id}`,
      nodes: executionNodes,
    });
    executionNodes = [];
  };

  nodes.forEach((node, index) => {
    if (isExecutionNode(node)) {
      executionNodes.push({ node, index });
      return;
    }
    flushExecution();
    items.push({ type: "message", node, index });
  });
  flushExecution();
  return items;
}

function TranscriptExecutionGroup({ group, expanded, onToggle, toolCount }) {
  const count = toolCount ?? executionToolCount(group.nodes.map(({ node }) => node));
  return (
    <>
      <button
        type="button"
        data-testid={`transcript-execution-group-${group.id}`}
        aria-expanded={expanded}
        onClick={onToggle}
        className="relative flex w-full items-center justify-center gap-2 py-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground hover:text-foreground transition-colors"
        title={expanded ? "collapse tool execution" : "expand tool execution"}
      >
        {expanded ? (
          <ChevronDown className="size-4" strokeWidth={1.75} />
        ) : (
          <MoreHorizontal className="size-4" strokeWidth={1.75} />
        )}
        <span>{expanded ? "collapse" : `${count} ${count === 1 ? "tool" : "tools"}`}</span>
      </button>
      <div className={`windie-reasoning-content ${expanded ? "open" : ""}`}>
        <div className="windie-reasoning-inner">
          {group.nodes.map(({ node, index }) => (
            <MessageRow key={node.id} node={node} index={index} isLast={false} />
          ))}
        </div>
      </div>
      {expanded ? (
        <button
          type="button"
          data-testid={`transcript-execution-collapse-bottom-${group.id}`}
          aria-expanded="true"
          onClick={onToggle}
          className="relative flex w-full items-center justify-center gap-2 py-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground hover:text-foreground transition-colors"
          title="collapse tool execution"
        >
          <ChevronUp className="size-4" strokeWidth={1.75} />
          <span>collapse</span>
        </button>
      ) : null}
    </>
  );
}

function LiveExecutionIndicator({ count }) {
  return (
    <div className="relative flex w-full items-center justify-center gap-2 py-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
      <MoreHorizontal className="size-4" strokeWidth={1.75} />
      <span>{count} {count === 1 ? "tool" : "tools"}</span>
    </div>
  );
}

export default function ChatPanel({ onFirstMessage }) {
  const { activeConv, selectedSession, selectedPathNodes, streaming, pendingAssistant, stopStreaming, apiError } = useWindie();
  const scrollRef = useRef(null);
  const prevConvId = useRef(activeConv?.id);
  const [expandedExecutionGroups, setExpandedExecutionGroups] = useState(() => new Set());
  const items = useMemo(() => transcriptItems(selectedPathNodes), [selectedPathNodes]);
  // Streaming previews belong to a session's active path, not to whichever
  // historical path is currently being inspected. Keep the session's
  // transient state alive in the hook, but hide it from alternate paths.
  const sessionHead = selectedSession?.currentHeadMessageId || null;
  const displayedHead = selectedPathNodes[selectedPathNodes.length - 1]?.id || null;
  const isViewingSessionHead = Boolean(
    activeConv?.id &&
      selectedSession?.conversationId === activeConv.id &&
      sessionHead &&
      displayedHead === sessionHead
  );
  const visiblePendingAssistant = isViewingSessionHead && streaming ? pendingAssistant : null;
  const pendingToolCount = visiblePendingAssistant?.toolCount || 0;
  const lastItem = items[items.length - 1];
  const currentExecutionGroup = lastItem?.type === "execution" ? lastItem : null;
  const persistedToolCount = currentExecutionGroup
    ? executionToolCount(currentExecutionGroup.nodes.map(({ node }) => node))
    : 0;
  const liveToolCount = pendingToolCount > persistedToolCount ? pendingToolCount : 0;

  // Scroll behavior:
  //   - On conversation switch: reset scroll to top (do NOT auto-scroll to bottom;
  //     that used to cause window/ancestor scroll on narrow viewports).
  //   - On new messages / streaming within the same conversation: pin to bottom.
  // We drive the scroll directly via scrollTop on our own container so the effect
  // never propagates to ancestor scroll contexts.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (prevConvId.current !== activeConv?.id) {
      el.scrollTop = 0;
      prevConvId.current = activeConv?.id;
    } else {
      el.scrollTop = el.scrollHeight;
    }
  }, [
    activeConv?.id,
    selectedPathNodes.length,
    streaming,
    pendingAssistant,
  ]);

  if (!activeConv) {
    return (
      <div
        data-testid="windie-welcome-canvas"
        className="relative flex-1 min-w-0 flex items-center overflow-hidden bg-background min-h-0"
      >
        <div className="pointer-events-none absolute inset-0 windie-welcome-grid" aria-hidden="true" />
        <div className="pointer-events-none absolute inset-0 overflow-hidden" aria-hidden="true">
          <div className="windie-welcome-orbit absolute right-[8%] top-1/2 size-[min(42vw,34rem)] -translate-y-1/2 rounded-full border border-border/60" />
          <div className="absolute right-[calc(8%+4rem)] top-1/2 size-[min(28vw,22rem)] -translate-y-1/2 rounded-full border border-border/40" />
          <div className="absolute right-[calc(8%+11rem)] top-1/2 size-2 -translate-y-1/2 rounded-full bg-accent shadow-[0_0_24px_hsl(var(--accent)/0.75)]" />
          <div className="absolute right-[calc(8%+4rem)] top-[calc(50%-1px)] h-px w-[min(42vw,34rem)] origin-left bg-border/50" />
          <div className="absolute right-[calc(8%+1px)] top-[calc(50%-min(21vw,17rem))] h-[min(42vw,34rem)] w-px bg-border/30" />
        </div>

        <div className="relative z-10 w-full max-w-5xl px-8 py-16 lg:px-16">
          <div className="grid items-center gap-16 lg:grid-cols-[minmax(0,1fr)_minmax(16rem,0.65fr)]">
            <div className="max-w-2xl">
              <div className="flex items-center gap-3 font-mono text-[10px] uppercase tracking-[0.28em] text-muted-foreground">
                <span className="size-2 bg-accent shadow-[0_0_12px_hsl(var(--accent)/0.65)]" />
                <span>local runtime / 001</span>
              </div>

              <h1 className="mt-8 text-4xl font-semibold tracking-[-0.05em] text-foreground sm:text-6xl lg:text-7xl">
                Welcome
                <span className="block text-muted-foreground">to Windie</span>
              </h1>

              <div className="mt-8 max-w-lg border-l border-accent pl-5">
                <p className="text-lg leading-relaxed text-foreground sm:text-xl">
                  This is Peter&apos;s creation of an AI on the computer
                </p>
                <p className="mt-3 font-mono text-xs uppercase tracking-[0.18em] text-muted-foreground">
                  Please be kind with it <span className="text-base normal-case tracking-normal">😊</span>
                </p>
              </div>

              {apiError ? (
                <div className="mt-10 max-w-lg border border-destructive/40 bg-destructive/5 px-3 py-2 font-mono text-[10px] text-destructive">
                  {apiError}
                </div>
              ) : null}
            </div>

            <div className="hidden justify-self-end lg:block">
              <div className="w-64 border border-border/80 bg-background/70 p-4 backdrop-blur-sm">
                <div className="flex items-center justify-between border-b border-border pb-3 font-mono text-[9px] uppercase tracking-[0.2em] text-muted-foreground">
                  <span>windie / core</span>
                  <span>01</span>
                </div>
                <div className="space-y-4 py-5 font-mono text-[10px] uppercase tracking-[0.16em]">
                  <div className="flex items-center justify-between">
                    <span className="text-muted-foreground">context</span>
                    <span className="text-foreground">ready</span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-muted-foreground">computer</span>
                    <span className="text-foreground">local</span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-muted-foreground">conversation</span>
                    <span className="text-accent">waiting</span>
                  </div>
                </div>
                <div className="flex items-center gap-2 border-t border-border pt-3 font-mono text-[9px] uppercase tracking-[0.16em] text-muted-foreground">
                  <span className="size-1.5 rounded-full bg-accent shadow-[0_0_10px_hsl(var(--accent)/0.7)]" />
                  <span>ready for a conversation</span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 min-w-0 flex flex-col bg-background min-h-0" data-testid="chat-panel">
      <div
        ref={scrollRef}
        data-testid="chat-scroll"
        className="flex-1 min-h-0 overflow-y-auto windie-scroll"
      >
        {items.map((item) => {
          if (item.type === "execution") {
            const expanded = expandedExecutionGroups.has(item.id);
            const toolCount = item === currentExecutionGroup && liveToolCount
              ? liveToolCount
              : undefined;
            return (
              <TranscriptExecutionGroup
                key={item.id}
                group={item}
                expanded={expanded}
                toolCount={toolCount}
                onToggle={() => setExpandedExecutionGroups((current) => {
                  const next = new Set(current);
                  if (next.has(item.id)) next.delete(item.id);
                  else next.add(item.id);
                  return next;
                })}
              />
            );
          }
          return (
            <MessageRow
              key={item.node.id}
              node={item.node}
              index={item.index}
              isLast={item.index === selectedPathNodes.length - 1}
            />
          );
        })}
        {!currentExecutionGroup && liveToolCount > 0 ? (
          <LiveExecutionIndicator count={liveToolCount} />
        ) : null}
        {visiblePendingAssistant && selectedSession ? (
          <PendingAssistantRow
            pendingAssistant={visiblePendingAssistant}
            index={selectedPathNodes.length}
            sessionId={selectedSession.id}
            onStop={() => stopStreaming(selectedSession.id)}
          />
        ) : null}
      </div>

      <ToolApprovalPrompt />
      <Composer onFirstMessage={onFirstMessage} />
    </div>
  );
}

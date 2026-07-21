import { useEffect, useMemo, useRef, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import MessageRow, { PendingAssistantRow } from "@/components/windie/MessageRow";
import Composer from "@/components/windie/Composer";
import { isExecutionNode } from "@/lib/treeProjection";
import { MoreHorizontal } from "lucide-react";

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

function TranscriptExecutionGroup({ group, expanded, onToggle }) {
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
        <MoreHorizontal className="size-4" strokeWidth={1.75} />
        <span>{expanded ? "collapse" : `${group.nodes.length} tools`}</span>
      </button>
      <div className={`windie-reasoning-content ${expanded ? "open" : ""}`}>
        <div className="windie-reasoning-inner">
          {group.nodes.map(({ node, index }) => (
            <MessageRow key={node.id} node={node} index={index} isLast={false} />
          ))}
        </div>
      </div>
    </>
  );
}

export default function ChatPanel() {
  const { activeConv, selectedSession, selectedPathNodes, streaming, pendingAssistant, stopStreaming, apiError } = useWindie();
  const scrollRef = useRef(null);
  const prevConvId = useRef(activeConv?.id);
  const [expandedExecutionGroups, setExpandedExecutionGroups] = useState(() => new Set());
  const items = useMemo(() => transcriptItems(selectedPathNodes), [selectedPathNodes]);

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
      <div className="flex-1 min-w-0 flex items-center justify-center bg-background min-h-0">
        <div className="font-mono text-xs text-muted-foreground">
          {apiError || "no conversation selected"}
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
            return (
              <TranscriptExecutionGroup
                key={item.id}
                group={item}
                expanded={expanded}
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
        {pendingAssistant && selectedSession ? (
          <PendingAssistantRow
            pendingAssistant={pendingAssistant}
            index={selectedPathNodes.length}
            sessionId={selectedSession.id}
            onStop={() => stopStreaming(selectedSession.id)}
          />
        ) : null}
      </div>

      <Composer />
    </div>
  );
}

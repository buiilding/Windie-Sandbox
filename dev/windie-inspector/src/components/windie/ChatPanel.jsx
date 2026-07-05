import { useEffect, useRef } from "react";
import { useWindie } from "@/context/WindieContext";
import MessageRow from "@/components/windie/MessageRow";
import Composer from "@/components/windie/Composer";

export default function ChatPanel() {
  const { activeConv, activePathNodes, streaming, apiError } = useWindie();
  const scrollRef = useRef(null);
  const prevConvId = useRef(activeConv?.id);

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
  }, [activeConv?.id, activePathNodes.length, streaming]);

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
      <div className="h-8 shrink-0 border-b border-border px-6 flex items-center justify-between font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        <div className="flex items-center gap-3">
          <span>active path</span>
          <span className="text-foreground/80">
            {activePathNodes.length} nodes
          </span>
          <span>·</span>
          <span>root {activeConv.rootId ? activeConv.rootId.slice(0, 6) : "(empty)"}</span>
          <span>·</span>
          <span>{Object.keys(activeConv.nodes).length} total</span>
        </div>
        <div className="flex items-center gap-3">
          <span>model</span>
          <span className="text-foreground/80">{activeConv.model}</span>
        </div>
      </div>

      <div
        ref={scrollRef}
        data-testid="chat-scroll"
        className="flex-1 min-h-0 overflow-y-auto windie-scroll"
      >
        {activePathNodes.map((node, i) => (
          <MessageRow
            key={node.id}
            node={node}
            index={i}
            isLast={i === activePathNodes.length - 1}
          />
        ))}
      </div>

      <Composer />
    </div>
  );
}

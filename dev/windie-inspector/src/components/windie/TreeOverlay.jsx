import { useMemo, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { ROLE_TOKENS } from "@/lib/mockData";
import { X, GitBranch, MoreHorizontal } from "lucide-react";
import ConversationTreeMenu from "@/components/windie/ConversationTreeMenu";
import TreeNodeContextMenu, { treeContextMenuPosition } from "@/components/windie/TreeNodeContextMenu";
import { isExecutionGroup, isExecutionNode, projectTree } from "@/lib/treeProjection";
import { layoutTree } from "@/lib/treeLayout";

export default function TreeOverlay() {
  const {
    activeConv,
    selectedPathNodes,
    setTreeOverlayOpen,
    selectedNodeId,
    setPathHead,
  } = useWindie();
  const [contextMenu, setContextMenu] = useState(null);
  const [expandedGroups, setExpandedGroups] = useState(new Set());

  const tree = useMemo(() => projectTree(activeConv, expandedGroups), [activeConv, expandedGroups]);
  const layout = useMemo(() => layoutTree(tree), [tree]);
  const pathSet = useMemo(
    () => new Set(selectedPathNodes.map((node) => node.id)),
    [selectedPathNodes]
  );
  const isProjectedNodeOnPath = (id) => {
    const node = tree.nodes[id];
    return node && (isExecutionGroup(node) ? node.hiddenIds.some((hiddenId) => pathSet.has(hiddenId)) : pathSet.has(node.originalId));
  };

  const toggleGroup = (groupId) => {
    setExpandedGroups((current) => {
      const next = new Set(current);
      if (next.has(groupId)) next.delete(groupId);
      else next.add(groupId);
      return next;
    });
  };

  return (
    <div
      data-testid="tree-overlay"
      className="absolute inset-0 z-30 bg-background/95 backdrop-blur-sm flex flex-col"
    >
      <div className="h-9 shrink-0 border-b border-border px-4 flex items-center justify-between font-mono text-[11px]">
        <div className="flex items-center gap-3">
          <GitBranch className="size-3.5" strokeWidth={1.75} />
          <span className="uppercase tracking-widest">conversation tree</span>
          <span className="text-muted-foreground">
            {Object.keys(activeConv.nodes).length} nodes ·{" "}
            {Object.values(activeConv.nodes).filter((n) => n.childrenIds.length > 1).length} branch
            points · selected path {selectedPathNodes.length}
          </span>
        </div>
        <div className="flex items-center gap-1">
          <ConversationTreeMenu />
          <button
            data-testid="tree-overlay-close"
            onClick={() => setTreeOverlayOpen(false)}
            className="p-1 border border-border hover:bg-surface-hover"
          >
            <X className="size-3.5" strokeWidth={1.75} />
          </button>
        </div>
      </div>

      <div className="flex-1 min-h-0 flex">
        {/* Canvas */}
        <div className="flex-1 min-w-0 overflow-auto windie-scroll windie-grid-bg">
          <div
            className="relative"
            style={{ width: layout.width, height: layout.height }}
          >
            <svg
              className="absolute inset-0 pointer-events-none"
              width={layout.width}
              height={layout.height}
            >
              {layout.edges.map(({ from, to }, i) => {
                const p1 = layout.positions[from];
                const p2 = layout.positions[to];
                if (!p1 || !p2) return null;
                const x1 = p1.x + layout.NODE_W / 2;
                const y1 = p1.y + p1.height;
                const x2 = p2.x + layout.NODE_W / 2;
                const y2 = p2.y;
                const active = isProjectedNodeOnPath(from) && isProjectedNodeOnPath(to);
                return (
                  <path
                    key={i}
                    d={`M ${x1} ${y1} C ${x1} ${(y1 + y2) / 2}, ${x2} ${(y1 + y2) / 2}, ${x2} ${y2}`}
                    stroke={
                      active
                        ? "hsl(var(--accent))"
                        : "hsl(var(--tree-edge))"
                    }
                    strokeWidth={active ? 1.75 : 1.5}
                    fill="none"
                    strokeLinecap="round"
                  />
                );
              })}
            </svg>

            {Object.entries(layout.positions).map(([id, pos]) => {
              const node = tree.nodes[id];
              if (!node) return null;
              const group = isExecutionGroup(node);
              const token = group ? null : ROLE_TOKENS[node.message.role];
              const onPath = group ? node.hiddenIds.some((hiddenId) => pathSet.has(hiddenId)) : pathSet.has(node.originalId);
              const isSel = !group && node.originalId === selectedNodeId;
              const text = group ? "" : node.message.parts.find((part) => part.type === "text")?.text || "";
              const className = `absolute text-left border transition-all duration-700 ease-out ${isExecutionNode(node) ? "windie-tree-execution-step" : ""} ${isSel ? "border-foreground bg-surface shadow-[0_0_0_1px_hsl(var(--foreground))]" : onPath ? "border-[hsl(var(--accent))]" : "border-border bg-background hover:border-foreground/60"}`;

              if (group) {
                return (
                  <button
                    key={id}
                    type="button"
                    data-testid={`tree-group-${id}`}
                    title={node.expanded ? "collapse tool execution" : "expand tool execution"}
                    onClick={() => toggleGroup(id)}
                    className="absolute flex items-center justify-center text-muted-foreground hover:text-foreground"
                    style={{ left: pos.x, top: pos.y, width: layout.NODE_W, height: pos.height }}
                  >
                    <div className="flex items-center justify-center gap-2 px-2">
                      <MoreHorizontal className="size-5 text-muted-foreground" strokeWidth={1.5} />
                      <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                        {node.expanded ? "collapse" : `${node.hiddenIds.length} tools`}
                      </span>
                    </div>
                  </button>
                );
              }

              return (
                <button
                  key={id}
                  type="button"
                  data-testid={`tree-node-${node.originalId}`}
                  onClick={() => {
                    setContextMenu(null);
                    setPathHead(node.originalId);
                  }}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    setContextMenu({
                      nodeId: node.originalId,
                      position: treeContextMenuPosition(event.clientX, event.clientY),
                    });
                  }}
                  className={className}
                  style={{
                    left: pos.x,
                    top: pos.y,
                    width: layout.NODE_W,
                    height: pos.height,
                  }}
                >
                  <div className="h-full flex flex-col p-2 gap-0.5">
                    <div className="flex items-center justify-between">
                      <span
                        className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}
                      >
                        [{token.label}]
                      </span>
                    </div>
                    <div className="font-mono text-[10px] text-muted-foreground truncate">
                      {node.message.model || " "}
                    </div>
                    <div className="windie-tree-preview text-[11px] leading-tight">
                      {text.slice(0, 180) || (
                        <span className="italic text-muted-foreground">(empty)</span>
                      )}
                    </div>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        {/* Side panel: selected node details */}
        <div className="w-80 shrink-0 border-l border-border flex flex-col">
          <div className="h-8 shrink-0 border-b border-border px-3 flex items-center font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            node inspector
          </div>
          <div className="flex-1 min-h-0 overflow-y-auto windie-scroll p-3 text-[11px]">
            {selectedNodeId && activeConv.nodes[selectedNodeId] ? (
              <TreeNodeDetail
                node={activeConv.nodes[selectedNodeId]}
                onPath={pathSet.has(selectedNodeId)}
              />
            ) : (
              <div className="text-muted-foreground font-mono">select a node</div>
            )}
          </div>
        </div>
      </div>
      <TreeNodeContextMenu
        nodeId={contextMenu?.nodeId}
        position={contextMenu?.position}
        onClose={() => setContextMenu(null)}
      />
    </div>
  );
}

function TreeNodeDetail({ node, onPath }) {
  const token = ROLE_TOKENS[node.message.role];
  const text = node.message.parts.find((p) => p.type === "text")?.text || "";
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <span className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}>
          [{token.label}]
        </span>
      </div>
      <div className="font-mono text-[10px] text-muted-foreground space-y-0.5">
        <div>parent: <span className="text-foreground">{node.parentId || "(root)"}</span></div>
        <div>children: <span className="text-foreground">{node.childrenIds.length}</span></div>
        {node.message.model && (
          <div>model: <span className="text-foreground">{node.message.model}</span></div>
        )}
        {node.message.tokens && (
          <div>tokens: <span className="text-foreground">{node.message.tokens}</span></div>
        )}
      </div>
      <div className="whitespace-pre-wrap leading-relaxed border-l-2 border-border pl-2 text-foreground/90">
        {text || <span className="italic text-muted-foreground">(empty)</span>}
      </div>

    </div>
  );
}

import { useMemo } from "react";
import { useWindie } from "@/context/WindieContext";
import { ROLE_TOKENS } from "@/lib/mockData";
import { X, GitBranch, Route, Scissors, Trash2 } from "lucide-react";
import { toast } from "sonner";

/**
 * Layout the tree by depth. For each depth level we place nodes horizontally.
 * Returns: {positions: {nodeId: {x, y, depth}}, width, height, edges: [{from,to}]}
 */
function layoutTree(conv) {
  const nodes = conv.nodes;
  const rootId = conv.rootId;
  // BFS depths
  const depthOf = {};
  const order = [];
  const queue = [rootId];
  depthOf[rootId] = 0;
  while (queue.length) {
    const id = queue.shift();
    order.push(id);
    (nodes[id]?.childrenIds || []).forEach((cid) => {
      if (depthOf[cid] === undefined) {
        depthOf[cid] = depthOf[id] + 1;
        queue.push(cid);
      }
    });
  }
  // group by depth
  const byDepth = {};
  order.forEach((id) => {
    const d = depthOf[id];
    if (!byDepth[d]) byDepth[d] = [];
    byDepth[d].push(id);
  });
  const NODE_W = 200;
  const NODE_H = 62;
  const H_GAP = 40;
  const V_GAP = 28;
  const positions = {};
  const maxRow = Math.max(...Object.values(byDepth).map((r) => r.length));
  Object.entries(byDepth).forEach(([d, ids]) => {
    ids.forEach((id, i) => {
      positions[id] = {
        x: 40 + i * (NODE_W + H_GAP),
        y: 40 + parseInt(d, 10) * (NODE_H + V_GAP),
        depth: parseInt(d, 10),
      };
    });
  });
  const edges = [];
  Object.values(nodes).forEach((n) => {
    if (n.parentId) edges.push({ from: n.parentId, to: n.id });
  });
  const width = Math.max(
    900,
    40 + maxRow * (NODE_W + H_GAP)
  );
  const height =
    Math.max(...Object.values(positions).map((p) => p.y)) + NODE_H + 40;
  return { positions, edges, width, height, NODE_W, NODE_H };
}

export default function TreeOverlay() {
  const {
    activeConv,
    setTreeOverlayOpen,
    selectedNodeId,
    setSelectedNodeId,
    setActivePathToLeaf,
    forkFromMessage,
    truncateAfter,
    removeMessage,
  } = useWindie();

  const layout = useMemo(() => layoutTree(activeConv), [activeConv]);
  const pathSet = useMemo(() => new Set(activeConv.activePath), [activeConv.activePath]);

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
            points · active path {activeConv.activePath.length}
          </span>
        </div>
        <button
          data-testid="tree-overlay-close"
          onClick={() => setTreeOverlayOpen(false)}
          className="p-1 border border-border hover:bg-surface-hover"
        >
          <X className="size-3.5" strokeWidth={1.75} />
        </button>
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
                const y1 = p1.y + layout.NODE_H;
                const x2 = p2.x + layout.NODE_W / 2;
                const y2 = p2.y;
                const active = pathSet.has(from) && pathSet.has(to);
                return (
                  <path
                    key={i}
                    d={`M ${x1} ${y1} C ${x1} ${(y1 + y2) / 2}, ${x2} ${(y1 + y2) / 2}, ${x2} ${y2}`}
                    stroke={
                      active
                        ? "hsl(var(--accent))"
                        : "hsl(var(--border))"
                    }
                    strokeWidth={active ? 1.5 : 1}
                    fill="none"
                    strokeDasharray={active ? "0" : "3 3"}
                  />
                );
              })}
            </svg>

            {Object.entries(layout.positions).map(([id, pos]) => {
              const n = activeConv.nodes[id];
              if (!n) return null;
              const role = n.message.role;
              const token = ROLE_TOKENS[role];
              const onPath = pathSet.has(id);
              const isSel = id === selectedNodeId;
              const text =
                n.message.parts.find((p) => p.type === "text")?.text || "";
              return (
                <button
                  key={id}
                  data-testid={`tree-node-${id}`}
                  onClick={() => setSelectedNodeId(id)}
                  className={`absolute text-left border transition-all ${
                    isSel
                      ? "border-foreground bg-surface shadow-[0_0_0_1px_hsl(var(--foreground))]"
                      : onPath
                        ? "border-[hsl(var(--accent))] bg-background"
                        : "border-border bg-background hover:border-foreground/60"
                  }`}
                  style={{
                    left: pos.x,
                    top: pos.y,
                    width: layout.NODE_W,
                    height: layout.NODE_H,
                  }}
                >
                  <div className="h-full flex flex-col p-2 gap-0.5">
                    <div className="flex items-center justify-between">
                      <span
                        className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}
                      >
                        [{token.label}]
                      </span>
                      <span className="font-mono text-[9px] text-muted-foreground">
                        {id.slice(0, 6)}
                      </span>
                    </div>
                    <div className="font-mono text-[10px] text-muted-foreground truncate">
                      {n.message.model || " "}
                    </div>
                    <div className="text-[11px] leading-tight truncate">
                      {text.slice(0, 42) || (
                        <span className="italic text-muted-foreground">(empty)</span>
                      )}
                    </div>
                    <div className="mt-auto flex items-center gap-2 font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                      {onPath && (
                        <span className="text-[hsl(var(--accent))]">on path</span>
                      )}
                      {n.childrenIds.length > 1 && (
                        <span className="text-foreground/80">
                          {n.childrenIds.length} branches
                        </span>
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
                onSetPath={() => {
                  setActivePathToLeaf(activeConv.id, selectedNodeId);
                  toast.message("active path set");
                }}
                onFork={() => {
                  forkFromMessage(activeConv.id, selectedNodeId);
                  toast.message("forked", { description: "new conversation created" });
                }}
                onTruncate={() => {
                  truncateAfter(activeConv.id, selectedNodeId);
                  toast.message("truncated", { description: "descendants deleted" });
                }}
                onRemove={() => {
                  removeMessage(activeConv.id, selectedNodeId);
                  toast.message("removed");
                }}
                isRoot={selectedNodeId === activeConv.rootId}
              />
            ) : (
              <div className="text-muted-foreground font-mono">select a node</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function TreeNodeDetail({ node, onPath, onSetPath, onFork, onTruncate, onRemove, isRoot }) {
  const token = ROLE_TOKENS[node.message.role];
  const text = node.message.parts.find((p) => p.type === "text")?.text || "";
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <span className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}>
          [{token.label}]
        </span>
        <span className="font-mono text-[10px] text-muted-foreground">{node.id}</span>
        {onPath && (
          <span className="ml-auto font-mono text-[9px] uppercase tracking-widest text-[hsl(var(--accent))]">
            on path
          </span>
        )}
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

      <div className="grid grid-cols-2 gap-1 pt-2 border-t border-border">
        <button
          data-testid="tree-detail-action-set-path"
          onClick={onSetPath}
          className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest"
        >
          <Route className="size-3" strokeWidth={1.75} /> set path
        </button>
        <button
          data-testid="tree-detail-action-fork"
          onClick={onFork}
          className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest"
        >
          <GitBranch className="size-3" strokeWidth={1.75} /> fork
        </button>
        <button
          data-testid="tree-detail-action-truncate"
          onClick={onTruncate}
          className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest"
        >
          <Scissors className="size-3" strokeWidth={1.75} /> truncate
        </button>
        <button
          data-testid="tree-detail-action-remove"
          disabled={isRoot}
          onClick={onRemove}
          className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))] disabled:opacity-40 disabled:cursor-not-allowed"
        >
          <Trash2 className="size-3" strokeWidth={1.75} /> remove
        </button>
      </div>
    </div>
  );
}

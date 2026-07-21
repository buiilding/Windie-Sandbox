import { useMemo, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { ROLE_TOKENS } from "@/lib/mockData";
import { GitBranch } from "lucide-react";
import ConversationTreeMenu from "@/components/windie/ConversationTreeMenu";
import TreeNodeContextMenu, { treeContextMenuPosition } from "@/components/windie/TreeNodeContextMenu";

function layoutTree(conv) {
  const nodes = conv.nodes;
  const rootIds = conv.rootIds?.length ? conv.rootIds : Object.values(nodes).filter((n) => n.parentId === null).map((n) => n.id);
  if (!rootIds.length) return { positions: {}, edges: [], width: 900, height: 280, NODE_W: 200, NODE_H: 62 };
  const depthOf = {};
  const order = [];
  const queue = [...rootIds];
  rootIds.forEach((r) => { depthOf[r] = 0; });
  while (queue.length) {
    const id = queue.shift();
    order.push(id);
    (nodes[id]?.childrenIds || []).forEach((cid) => {
      if (depthOf[cid] === undefined) { depthOf[cid] = depthOf[id] + 1; queue.push(cid); }
    });
  }
  const byDepth = {};
  order.forEach((id) => { const d = depthOf[id]; if (!byDepth[d]) byDepth[d] = []; byDepth[d].push(id); });
  const NODE_W = 200, NODE_H = 62, H_GAP = 40, V_GAP = 28;
  const positions = {};
  const maxRow = Math.max(...Object.values(byDepth).map((r) => r.length), 1);
  Object.entries(byDepth).forEach(([d, ids]) => { ids.forEach((id, i) => { positions[id] = { x: 40 + i * (NODE_W + H_GAP), y: 40 + parseInt(d, 10) * (NODE_H + V_GAP), depth: parseInt(d, 10) }; }); });
  const edges = [];
  Object.values(nodes).forEach((n) => { if (n.parentId && nodes[n.parentId]) edges.push({ from: n.parentId, to: n.id }); });
  const width = Math.max(900, 40 + maxRow * (NODE_W + H_GAP));
  const height = Math.max(...Object.values(positions).map((p) => p.y), 0) + NODE_H + 40;
  return { positions, edges, width, height, NODE_W, NODE_H };
}

export default function TreePanel() {
  const { activeConv, selectedPathNodes, selectedNodeId, setPathHead } = useWindie();
  const [contextMenu, setContextMenu] = useState(null);
  const layout = useMemo(() => (activeConv ? layoutTree(activeConv) : null), [activeConv]);
  const pathSet = useMemo(() => new Set(selectedPathNodes.map((n) => n.id)), [selectedPathNodes]);

  if (!activeConv || !layout) {
    return <div className="h-full w-full flex items-center justify-center font-mono text-[11px] text-muted-foreground">no conversation</div>;
  }

  return (
    <div className="h-full w-full flex flex-col bg-background">
      <div className="h-9 shrink-0 border-b border-border px-4 flex items-center justify-between font-mono text-[11px]">
        <div className="flex items-center gap-3 min-w-0">
          <GitBranch className="size-3.5" />
          <span className="uppercase tracking-widest">conversation tree</span>
          <span className="text-muted-foreground truncate">{Object.keys(activeConv.nodes).length} nodes · {Object.values(activeConv.nodes).filter((n) => n.childrenIds.length > 1).length} branches · path {selectedPathNodes.length}</span>
        </div>
        <ConversationTreeMenu />
      </div>
      <div className="flex-1 min-h-0 overflow-auto windie-scroll windie-grid-bg">
        <div className="relative" style={{ width: layout.width, height: layout.height }}>
          <svg className="absolute inset-0 pointer-events-none" width={layout.width} height={layout.height}>
            {layout.edges.map(({ from, to }, i) => {
              const p1 = layout.positions[from], p2 = layout.positions[to];
              if (!p1 || !p2) return null;
              return <path key={i} d={`M ${p1.x + layout.NODE_W / 2} ${p1.y + layout.NODE_H} C ${p1.x + layout.NODE_W / 2} ${(p1.y + layout.NODE_H + p2.y) / 2}, ${p2.x + layout.NODE_W / 2} ${(p1.y + layout.NODE_H + p2.y) / 2}, ${p2.x + layout.NODE_W / 2} ${p2.y}`} stroke={pathSet.has(from) && pathSet.has(to) ? "hsl(var(--accent))" : "hsl(var(--border))"} strokeWidth={pathSet.has(from) && pathSet.has(to) ? 1.5 : 1} fill="none" strokeDasharray={pathSet.has(from) && pathSet.has(to) ? "0" : "3 3"} />;
            })}
          </svg>
          {Object.entries(layout.positions).map(([id, pos]) => {
            const n = activeConv.nodes[id];
            if (!n) return null;
            const token = ROLE_TOKENS[n.message.role];
            const onPath = pathSet.has(id);
            const isSel = id === selectedNodeId;
            const text = n.message.parts.find((p) => p.type === "text")?.text || "";
            return (
              <button key={id} data-testid={`tree-node-${id}`} onClick={() => { setContextMenu(null); setPathHead(id); }} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ nodeId: id, position: treeContextMenuPosition(event.clientX, event.clientY) }); }} className={`absolute text-left border transition-all ${isSel ? "border-foreground bg-surface shadow-[0_0_0_1px_hsl(var(--foreground))]" : onPath ? "border-[hsl(var(--accent))] bg-background" : "border-border bg-background hover:border-foreground/60"}`} style={{ left: pos.x, top: pos.y, width: layout.NODE_W, height: layout.NODE_H }}>
                <div className="h-full flex flex-col p-2 gap-0.5">
                  <div className="flex items-center justify-between"><span className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}>[{token.label}]</span><span className="font-mono text-[9px] text-muted-foreground">{id.slice(0, 6)}</span></div>
                  <div className="text-[11px] leading-tight truncate">{text.slice(0, 42) || <span className="italic text-muted-foreground">(empty)</span>}</div>
                  <div className="mt-auto flex gap-2 font-mono text-[9px] uppercase tracking-widest text-muted-foreground">{onPath && <span className="text-[hsl(var(--accent))]">on path</span>}{n.childrenIds.length > 1 && <span>{n.childrenIds.length} branches</span>}</div>
                </div>
              </button>
            );
          })}
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

import { useMemo, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { ROLE_TOKENS } from "@/lib/mockData";
import { GitBranch, MoreHorizontal } from "lucide-react";
import ConversationTreeMenu from "@/components/windie/ConversationTreeMenu";
import TreeNodeContextMenu, { treeContextMenuPosition } from "@/components/windie/TreeNodeContextMenu";
import { isExecutionGroup, projectTree } from "@/lib/treeProjection";

function layoutTree(tree) {
  const nodes = tree.nodes;
  const rootIds = tree.rootIds;
  if (!rootIds.length) return { positions: {}, edges: [], width: 900, height: 280, NODE_W: 200, NODE_H: 62 };

  const depthOf = {};
  const order = [];
  const queue = [...rootIds];
  rootIds.forEach((rootId) => { depthOf[rootId] = 0; });
  while (queue.length) {
    const id = queue.shift();
    order.push(id);
    (nodes[id]?.childrenIds || []).forEach((childId) => {
      if (depthOf[childId] === undefined) {
        depthOf[childId] = depthOf[id] + 1;
        queue.push(childId);
      }
    });
  }

  const byDepth = {};
  order.forEach((id) => {
    const depth = depthOf[id];
    if (!byDepth[depth]) byDepth[depth] = [];
    byDepth[depth].push(id);
  });

  const NODE_W = 200;
  const NODE_H = 62;
  const GROUP_H = 30;
  const H_GAP = 40;
  const V_GAP = 28;
  const positions = {};
  const maxRow = Math.max(...Object.values(byDepth).map((row) => row.length), 1);
  let y = 40;
  Object.entries(byDepth).forEach(([depth, ids]) => {
    const rowHeight = Math.max(...ids.map((id) => (isExecutionGroup(nodes[id]) ? GROUP_H : NODE_H)));
    ids.forEach((id, index) => {
      positions[id] = {
        x: 40 + index * (NODE_W + H_GAP),
        y,
        depth: Number(depth),
        height: isExecutionGroup(nodes[id]) ? GROUP_H : NODE_H,
      };
    });
    y += rowHeight + V_GAP;
  });

  const edges = [];
  Object.values(nodes).forEach((node) => {
    (node.childrenIds || []).forEach((childId) => {
      if (nodes[childId]) edges.push({ from: node.id, to: childId });
    });
  });
  const width = Math.max(900, 40 + maxRow * (NODE_W + H_GAP));
  const height = Math.max(...Object.values(positions).map((position) => position.y + position.height), 0) + 40;
  return { positions, edges, width, height, NODE_W, NODE_H };
}

export default function TreePanel() {
  const { activeConv, selectedPathNodes, selectedNodeId, setPathHead } = useWindie();
  const [contextMenu, setContextMenu] = useState(null);
  const [expandedGroups, setExpandedGroups] = useState(new Set());
  const tree = useMemo(() => projectTree(activeConv, expandedGroups), [activeConv, expandedGroups]);
  const layout = useMemo(() => layoutTree(tree), [tree]);
  const pathSet = useMemo(() => new Set(selectedPathNodes.map((node) => node.id)), [selectedPathNodes]);
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

  if (!activeConv) {
    return <div className="h-full w-full flex items-center justify-center font-mono text-[11px] text-muted-foreground">no conversation</div>;
  }

  return (
    <div className="h-full w-full flex flex-col bg-background">
      <div className="h-9 shrink-0 border-b border-border px-4 flex items-center justify-between font-mono text-[11px]">
        <div className="flex items-center gap-3 min-w-0">
          <GitBranch className="size-3.5" />
          <span className="uppercase tracking-widest">conversation tree</span>
          <span className="text-muted-foreground truncate">{Object.keys(activeConv.nodes).length} nodes · {Object.values(activeConv.nodes).filter((node) => node.childrenIds.length > 1).length} branches · path {selectedPathNodes.length}</span>
        </div>
        <ConversationTreeMenu />
      </div>
      <div className="flex-1 min-h-0 overflow-auto windie-scroll windie-grid-bg">
        <div className="relative" style={{ width: layout.width, height: layout.height }}>
          <svg className="absolute inset-0 pointer-events-none" width={layout.width} height={layout.height}>
            {layout.edges.map(({ from, to }, index) => {
              const fromPosition = layout.positions[from];
              const toPosition = layout.positions[to];
              if (!fromPosition || !toPosition) return null;
              const active = isProjectedNodeOnPath(from) && isProjectedNodeOnPath(to);
              return <path key={index} d={`M ${fromPosition.x + layout.NODE_W / 2} ${fromPosition.y + fromPosition.height} C ${fromPosition.x + layout.NODE_W / 2} ${(fromPosition.y + fromPosition.height + toPosition.y) / 2}, ${toPosition.x + layout.NODE_W / 2} ${(fromPosition.y + fromPosition.height + toPosition.y) / 2}, ${toPosition.x + layout.NODE_W / 2} ${toPosition.y}`} stroke={active ? "hsl(var(--accent))" : "hsl(var(--border))"} strokeWidth={active ? 1.5 : 1} fill="none" strokeDasharray={active ? "0" : "3 3"} />;
            })}
          </svg>
          {Object.entries(layout.positions).map(([id, position]) => {
            const node = tree.nodes[id];
            if (!node) return null;
            const group = isExecutionGroup(node);
            const onPath = group ? node.hiddenIds.some((hiddenId) => pathSet.has(hiddenId)) : pathSet.has(node.originalId);
            const isSelected = !group && node.originalId === selectedNodeId;
            const token = group ? null : ROLE_TOKENS[node.message.role];
            const text = group ? "" : node.message.parts.find((part) => part.type === "text")?.text || "";
            const className = `absolute text-left border transition-all ${isSelected ? "border-foreground bg-surface shadow-[0_0_0_1px_hsl(var(--foreground))]" : onPath ? "border-[hsl(var(--accent))] bg-background" : "border-border bg-background hover:border-foreground/60"}`;

            if (group) {
              return (
                <button
                  key={id}
                  type="button"
                  data-testid={`tree-group-${id}`}
                  title="expand tool execution"
                  onClick={() => toggleGroup(id)}
                  className="absolute flex items-center justify-center text-muted-foreground hover:text-foreground"
                  style={{ left: position.x, top: position.y, width: layout.NODE_W, height: position.height }}
                >
                  <div className="flex items-center justify-center gap-2 px-2">
                    <MoreHorizontal className="size-5 text-muted-foreground" strokeWidth={1.5} />
                    <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                      {node.hiddenIds.length} tools
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
                onClick={() => { setContextMenu(null); setPathHead(node.originalId); }}
                onContextMenu={(event) => { event.preventDefault(); setContextMenu({ nodeId: node.originalId, position: treeContextMenuPosition(event.clientX, event.clientY) }); }}
                className={className}
                style={{ left: position.x, top: position.y, width: layout.NODE_W, height: position.height }}
              >
                <div className="h-full flex flex-col p-2 gap-0.5">
                  <div className="flex items-center justify-between"><span className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}>[{token.label}]</span><span className="font-mono text-[9px] text-muted-foreground">{node.originalId.slice(0, 6)}</span></div>
                  <div className="text-[11px] leading-tight truncate">{text.slice(0, 42) || <span className="italic text-muted-foreground">(empty)</span>}</div>
                  <div className="mt-auto flex gap-2 font-mono text-[9px] uppercase tracking-widest text-muted-foreground">{onPath && <span className="text-[hsl(var(--accent))]">on path</span>}{node.childrenIds.length > 1 && <span>{node.childrenIds.length} branches</span>}</div>
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

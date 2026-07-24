import { useMemo, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { ROLE_TOKENS } from "@/lib/mockData";
import { MoreHorizontal } from "lucide-react";
import TreeNodeContextMenu, { treeContextMenuPosition } from "@/components/windie/TreeNodeContextMenu";
import { isExecutionGroup, isExecutionNode, projectTree } from "@/lib/treeProjection";
import { layoutTree } from "@/lib/treeLayout";

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
      <div className="flex-1 min-h-0 overflow-auto windie-scroll windie-grid-bg">
        <div className="relative" style={{ width: layout.width, height: layout.height }}>
          <svg className="absolute inset-0 pointer-events-none" width={layout.width} height={layout.height}>
            {layout.edges.map(({ from, to }, index) => {
              const fromPosition = layout.positions[from];
              const toPosition = layout.positions[to];
              if (!fromPosition || !toPosition) return null;
              const active = isProjectedNodeOnPath(from) && isProjectedNodeOnPath(to);
              return <path key={index} d={`M ${fromPosition.x + layout.NODE_W / 2} ${fromPosition.y + fromPosition.height} C ${fromPosition.x + layout.NODE_W / 2} ${(fromPosition.y + fromPosition.height + toPosition.y) / 2}, ${toPosition.x + layout.NODE_W / 2} ${(fromPosition.y + fromPosition.height + toPosition.y) / 2}, ${toPosition.x + layout.NODE_W / 2} ${toPosition.y}`} stroke={active ? "hsl(var(--accent))" : "hsl(var(--tree-edge))"} strokeWidth={active ? 1.75 : 1.5} fill="none" strokeLinecap="round" />;
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
            const className = `absolute text-left border transition-all duration-700 ease-out ${isExecutionNode(node) ? "windie-tree-execution-step" : ""} ${isSelected ? "border-foreground bg-surface shadow-[0_0_0_1px_hsl(var(--foreground))]" : onPath ? "border-[hsl(var(--accent))]" : "border-border bg-background hover:border-foreground/60"}`;

            if (group) {
              return (
                <button
                  key={id}
                  type="button"
                  data-testid={`tree-group-${id}`}
                  title={node.expanded ? "collapse tool execution" : "expand tool execution"}
                  onClick={() => toggleGroup(id)}
                  className="absolute flex items-center justify-center text-muted-foreground hover:text-foreground"
                  style={{ left: position.x, top: position.y, width: layout.NODE_W, height: position.height }}
                >
                  <div className="flex items-center justify-center gap-2 px-2">
                      <MoreHorizontal className="size-5 text-muted-foreground" strokeWidth={1.5} />
                      <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                        {node.expanded ? "collapse" : `${node.toolCount} ${node.toolCount === 1 ? "tool" : "tools"}`}
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
                  <div className="flex items-center"><span className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}>[{token.label}]</span></div>
                  <div className="windie-tree-preview text-[11px] leading-tight">{text.slice(0, 180) || <span className="italic text-muted-foreground">(empty)</span>}</div>
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

import { useState, useMemo, useEffect, useRef } from "react";
import { useWindie } from "@/context/WindieContext";
import { ROLE_TOKENS } from "@/lib/mockData";
import {
  ChevronRight,
  Pencil,
  GitBranch,
  Scissors,
  Trash2,
  ChevronDown,
  Route,
  Plus,
  Loader2,
} from "lucide-react";
import { toast } from "sonner";

function Section({ title, children, defaultOpen = true, right, testId, resetKey }) {
  const [open, setOpen] = useState(defaultOpen);
  useEffect(() => {
    if (resetKey === undefined) return;
    setOpen(defaultOpen);
  }, [defaultOpen, resetKey]);
  return (
    <div className="border-b border-border" data-testid={testId}>
      <button onClick={() => setOpen(!open)} className="w-full px-3 py-2 flex items-center justify-between hover:bg-surface/60">
        <div className="flex items-center gap-1.5">
          {open ? <ChevronDown className="size-3 text-muted-foreground" /> : <ChevronRight className="size-3 text-muted-foreground" />}
          <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{title}</span>
        </div>
        {right}
      </button>
      {open && <div className="px-3 pb-3">{children}</div>}
    </div>
  );
}

function KV({ k, v, mono = true }) {
  return (
    <div className="flex items-baseline gap-2 py-0.5 text-[11px]">
      <span className="text-muted-foreground font-mono uppercase tracking-widest w-24 shrink-0">{k}</span>
      <span className={mono ? "font-mono text-foreground break-all" : "text-foreground"}>{v}</span>
    </div>
  );
}

function isAncestor(ancestorId, leafId, nodes) {
  if (!ancestorId || !leafId || !nodes) return false;
  if (ancestorId === leafId) return true;
  let cur = nodes[leafId];
  const seen = new Set();
  while (cur && !seen.has(cur.id)) {
    seen.add(cur.id);
    if (cur.parentId === ancestorId) return true;
    cur = cur.parentId ? nodes[cur.parentId] : null;
  }
  return false;
}

export default function InspectorPanel() {
  const {
    activeConv,
    selectedNodeId,
    selectedPathNodes,
    setSystemPrompt,
    setToolApprovalMode,
    forkFromMessage,
    truncateAfter,
    removeMessage,
    toolSchemas,
    approvals,
    approveToolCall,
    denyToolCall,
    availableToolSchemas,
    addToolSchema,
    addToolSchemas,
    removeToolSchema,
    removeToolSchemas,
    toolProviderStatuses,
    sessionsById,
    subscribeToPathLeaf,
    inspectNode,
  } = useWindie();

  const [editingSys, setEditingSys] = useState(false);
  const [sysDraft, setSysDraft] = useState(activeConv?.systemPrompt || "");
  const [pendingToolActionKeys, setPendingToolActionKeys] = useState([]);
  const [collapsedToolProviderIds, setCollapsedToolProviderIds] = useState([]);
  const pendingRef = useRef(new Set());
  const initRef = useRef(new Set());

  useEffect(() => setSysDraft(activeConv?.systemPrompt || ""), [activeConv?.id, activeConv?.systemPrompt]);

  const selectedNode = selectedNodeId ? activeConv?.nodes[selectedNodeId] : null;
  const selectedPathIds = useMemo(() => new Set(selectedPathNodes.map((n) => n.id)), [selectedPathNodes]);
  const onSelectedPath = selectedNode && selectedPathIds.has(selectedNodeId);
  const attachedNames = useMemo(() => new Set(toolSchemas.map((s) => s.name)), [toolSchemas]);
  const pendingSet = useMemo(() => new Set(pendingToolActionKeys), [pendingToolActionKeys]);
  const collapsedSet = useMemo(() => new Set(collapsedToolProviderIds), [collapsedToolProviderIds]);

  const toggle = (id) => setCollapsedToolProviderIds((c) => (c.includes(id) ? c.filter((x) => x !== id) : [...c, id]));
  const setPending = (k, v) => {
    const n = new Set(pendingRef.current);
    if (v) n.add(k);
    else n.delete(k);
    pendingRef.current = n;
    setPendingToolActionKeys([...n]);
  };
  const runAction = async (k, action, msg, desc) => {
    if (pendingRef.current.has(k)) return;
    setPending(k, true);
    try {
      await action();
      toast.message(msg, desc ? { description: desc } : undefined);
    } finally {
      setPending(k, false);
    }
  };

  const grouped = useMemo(() => {
    const groups = [];
    const byId = new Map();
    for (const s of availableToolSchemas) {
      const pid = s.providerId || "unknown";
      let g = byId.get(pid);
      if (!g) {
        g = { providerId: pid, tools: [] };
        byId.set(pid, g);
        groups.push(g);
      }
      g.tools.push(s);
    }
    return groups;
  }, [availableToolSchemas]);

  const unavailable = useMemo(() => (toolProviderStatuses || []).filter((p) => !p.available), [toolProviderStatuses]);

  useEffect(() => {
    const unseen = grouped.map((g) => g.providerId).filter((id) => !initRef.current.has(id));
    if (!unseen.length) return;
    unseen.forEach((id) => initRef.current.add(id));
    setCollapsedToolProviderIds((c) => [...new Set([...c, ...unseen])]);
  }, [grouped]);

  if (!activeConv) return null;

  return (
    <aside data-testid="windie-inspector" className="w-[340px] shrink-0 border-l border-border bg-background flex flex-col">
      <div className="h-8 shrink-0 border-b border-border px-3 flex items-center font-mono text-[10px] uppercase tracking-widest text-muted-foreground">inspector</div>
      <div className="flex-1 min-h-0 overflow-y-auto windie-scroll" style={{ scrollbarGutter: "stable" }}>
        <Section title="conversation" testId="inspector-section-conversation">
          <KV k="id" v={activeConv.id} />
          <KV k="model" v={activeConv.model} />
          <KV k="nodes" v={Object.keys(activeConv.nodes).length} />
          <KV k="branches" v={Object.values(activeConv.nodes).filter((n) => n.childrenIds.length > 1).length} />
          <KV k="updated" v={new Date(activeConv.updatedAt).toLocaleString()} />
          <div className="flex items-center gap-2 py-1 text-[11px]">
            <span className="text-muted-foreground font-mono uppercase tracking-widest w-24 shrink-0">tool access</span>
            <div className="grid grid-cols-2 border border-border">
              <button data-testid="tool-approval-mode-manual" onClick={() => { setToolApprovalMode(activeConv.id, "manual"); toast.message("tool access set", { description: "manual" }); }} className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest ${activeConv.toolApprovalMode === "manual" ? "bg-foreground text-background" : "text-muted-foreground hover:bg-surface-hover"}`}>manual</button>
              <button data-testid="tool-approval-mode-auto" onClick={() => { setToolApprovalMode(activeConv.id, "auto_approve_attached"); toast.message("tool access set", { description: "full access" }); }} className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest border-l border-border ${activeConv.toolApprovalMode === "auto_approve_attached" ? "bg-foreground text-background" : "text-muted-foreground hover:bg-surface-hover"}`}>full access</button>
            </div>
          </div>
        </Section>

        <Section title={`paths · ${(activeConv?.paths || []).length}`} testId="inspector-section-paths" defaultOpen={false}>
          {(activeConv?.paths || []).length === 0 ? (
            <div className="font-mono text-[11px] text-muted-foreground">no paths</div>
          ) : (
            <div className="space-y-0.5">
              {activeConv.paths.map((path, idx) => {
                const leafId = path.leafMessageId;
                if (!leafId) return null;
                const isCurrent = activeConv.selectedPath?.includes(leafId);
                let running = false;
                if (activeConv.nodes && sessionsById) {
                  for (const s of Object.values(sessionsById)) {
                    if (s.status !== "running") continue;
                    if (s.conversationId !== activeConv.id) continue;
                    if (isAncestor(s.startHeadMessageId, leafId, activeConv.nodes) || s.startHeadMessageId === leafId) {
                      running = true;
                      break;
                    }
                  }
                }
                return (
                  <button key={leafId} data-testid={`inspector-path-leaf-${leafId}`} onClick={() => { subscribeToPathLeaf(leafId, activeConv.id); toast.message("path subscribed", { description: `${path.leafPreview?.slice(0, 40) || ""}${running ? " · running" : ""}` }); }} className={`w-full text-left flex items-center gap-2 px-1.5 py-1 border-l-2 font-mono text-[11px] ${isCurrent ? "bg-surface border-[hsl(var(--accent))]" : "border-transparent hover:bg-surface/60"}`}>
                    <span className="text-muted-foreground w-5 text-right">{String(idx).padStart(2, "0")}</span>
                    <span className="text-muted-foreground w-10">d{path.depth}</span>
                    {running && <span className="size-1.5 bg-green-500 rounded-full shrink-0" />}
                    <span className="truncate flex-1 text-muted-foreground">{path.leafPreview || "(empty)"}</span>
                  </button>
                );
              })}
            </div>
          )}
        </Section>

        <Section title="selected path" testId="inspector-section-active-path" defaultOpen={false}>
          <div className="font-mono text-[10px] text-muted-foreground mb-1.5">{selectedPathNodes.length} nodes · click to inspect (unfollow)</div>
          <div className="space-y-0.5">
            {selectedPathNodes.map((n, i) => {
              if (!n) return null;
              const token = ROLE_TOKENS[n.message.role];
              return (
                <button key={n.id} data-testid={`inspector-path-node-${n.id}`} onClick={() => inspectNode(n.id)} className={`w-full text-left flex items-center gap-2 px-1.5 py-1 border-l-2 font-mono text-[11px] ${n.id === selectedNodeId ? "bg-surface border-[hsl(var(--accent))]" : "border-transparent hover:bg-surface/60"}`}>
                  <span className="text-muted-foreground w-5 text-right">{String(i).padStart(2, "0")}</span>
                  <span className={`w-8 ${token.color}`}>[{token.label}]</span>
                  <span className="truncate flex-1 text-muted-foreground">{(n.message.parts.find((p) => p.type === "text")?.text || "").slice(0, 40) || "(empty)"}</span>
                </button>
              );
            })}
          </div>
        </Section>

        <Section title="system prompt" testId="inspector-section-system-prompt" defaultOpen={false} right={!editingSys && <span data-testid="inspector-edit-sysprompt-icon" onClick={(e) => { e.stopPropagation(); setSysDraft(activeConv.systemPrompt); setEditingSys(true); }} className="p-1 hover:bg-surface-hover"><Pencil className="size-3" /></span>}>
          {editingSys ? (
            <div className="space-y-2">
              <textarea data-testid="inspector-sysprompt-textarea" value={sysDraft} onChange={(e) => setSysDraft(e.target.value)} rows={5} className="w-full bg-transparent border border-foreground/60 p-2 font-mono text-[11px] outline-none resize-none" />
              <div className="flex gap-1">
                <button data-testid="inspector-sysprompt-commit" onClick={() => { setSystemPrompt(activeConv.id, sysDraft); setEditingSys(false); toast.message("system prompt updated"); }} className="text-[10px] uppercase px-2 py-1 border border-foreground bg-foreground text-background font-mono">commit</button>
                <button onClick={() => setEditingSys(false)} className="text-[10px] uppercase px-2 py-1 border border-border font-mono">cancel</button>
              </div>
            </div>
          ) : (
            <div className="font-mono text-[11px] text-muted-foreground whitespace-pre-wrap border-l-2 border-muted-foreground/40 pl-2 py-1">{activeConv.systemPrompt}</div>
          )}
        </Section>

        <Section title={selectedNodeId ? `selected · ${activeConv.nodes[selectedNodeId] ? ROLE_TOKENS[activeConv.nodes[selectedNodeId].message.role].label : ""}` : "selected message"} testId="inspector-section-selected">
          {!selectedNodeId || !activeConv.nodes[selectedNodeId] ? (
            <div className="font-mono text-[11px] text-muted-foreground">no message selected</div>
          ) : (
            <>
              <KV k="node id" v={selectedNodeId} />
              <KV k="parent" v={activeConv.nodes[selectedNodeId].parentId || "(root)"} />
              <KV k="children" v={activeConv.nodes[selectedNodeId].childrenIds.length} />
              <KV k="on path" v={selectedPathIds.has(selectedNodeId) ? <span className="text-[hsl(var(--accent))]">yes</span> : <span className="text-muted-foreground">no</span>} />
              <div className="mt-3 grid grid-cols-2 gap-1">
                <button data-testid="inspector-action-fork" onClick={() => { forkFromMessage(activeConv.id, selectedNodeId); toast.message("forked"); }} className="h-8 flex items-center justify-center gap-1.5 border border-border font-mono text-[10px] uppercase tracking-widest"><GitBranch className="size-3" /> fork</button>
                <button data-testid="inspector-action-set-path" onClick={() => { subscribeToPathLeaf(selectedNodeId, activeConv.id); toast.message("path subscribed"); }} className="h-8 flex items-center justify-center gap-1.5 border border-border font-mono text-[10px] uppercase tracking-widest"><Route className="size-3" /> subscribe</button>
                <button data-testid="inspector-action-truncate" onClick={() => { truncateAfter(activeConv.id, selectedNodeId); toast.message("truncated"); }} className="h-8 flex items-center justify-center gap-1.5 border border-border font-mono text-[10px] uppercase tracking-widest"><Scissors className="size-3" /> truncate</button>
                <button data-testid="inspector-action-remove" onClick={() => { removeMessage(activeConv.id, selectedNodeId); toast.message("removed"); }} className="h-8 flex items-center justify-center gap-1.5 border border-border font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))]"><Trash2 className="size-3" /> remove</button>
              </div>
            </>
          )}
        </Section>

        <Section title={`approvals · ${approvals.length}`} testId="inspector-section-approvals" defaultOpen={true}>
          {approvals.length === 0 ? <div className="font-mono text-[11px] text-muted-foreground">no pending</div> : approvals.map((a) => (
            <div key={a.tool_call_id} className="border border-border mb-2">
              <div className="px-2 py-1 border-b border-border flex justify-between"><span className="font-mono text-[11px] text-[hsl(var(--tool-call))]">{a.tool_name}</span><span className="font-mono text-[9px] text-muted-foreground">session {a.session_id?.slice(0, 8)}</span></div>
              <div className="px-2 py-1.5 space-y-1">
                <div className="font-mono text-[10px] text-muted-foreground">{a.reason}</div>
                <pre className="font-mono text-[10px] bg-surface/60 border border-border p-2 overflow-auto max-h-32 whitespace-pre-wrap">{(() => { try { return JSON.stringify(JSON.parse(a.arguments), null, 2); } catch { return a.arguments; } })()}</pre>
                <div className="grid grid-cols-3 gap-1">
                  <button data-testid={`approval-view-${a.tool_call_id}`} onClick={() => { inspectNode(a.assistant_message_id); }} className="h-7 border border-border font-mono text-[10px] uppercase">view path</button>
                  <button data-testid={`approval-approve-${a.tool_call_id}`} onClick={() => { approveToolCall(a.session_id, a.tool_call_id); toast.message("approved"); }} className="h-7 border border-foreground bg-foreground text-background font-mono text-[10px] uppercase">approve</button>
                  <button data-testid={`approval-deny-${a.tool_call_id}`} onClick={() => { denyToolCall(a.session_id, a.tool_call_id); toast.message("denied"); }} className="h-7 border border-border font-mono text-[10px] uppercase text-[hsl(var(--destructive))]">deny</button>
                </div>
              </div>
            </div>
          ))}
        </Section>

        <Section title={`tool schemas · ${availableToolSchemas.length}`} testId="inspector-section-tools" defaultOpen={true} resetKey={activeConv.id}>
          <div className="space-y-2">
            {grouped.length > 0 || unavailable.length > 0 ? (
              <div className="space-y-2">
                {grouped.map((g) => {
                  const unatt = g.tools.filter((s) => !attachedNames.has(s.name));
                  const att = g.tools.filter((s) => attachedNames.has(s.name));
                  const addK = `provider:add:${activeConv.id}:${g.providerId}`;
                  const remK = `provider:remove:${activeConv.id}:${g.providerId}`;
                  const addP = pendingSet.has(addK);
                  const remP = pendingSet.has(remK);
                  const pend = addP || remP;
                  const coll = collapsedSet.has(g.providerId);
                  return (
                    <div key={g.providerId} className="border border-border">
                      <div role="button" tabIndex={0} onClick={() => toggle(g.providerId)} className={`w-full min-h-8 px-2 py-1.5 flex items-center justify-between gap-2 bg-surface/40 hover:bg-surface-hover ${coll ? "" : "border-b border-border"}`}>
                        <div className="min-w-0 text-left"><div className="font-mono text-[10px] uppercase">{g.providerId}</div><div className="font-mono text-[10px] text-muted-foreground">{g.tools.length} tools</div></div>
                        <div className="flex gap-1">
                          {unatt.length > 0 && <button disabled={pend} onClick={(e) => { e.stopPropagation(); runAction(addK, () => addToolSchemas(activeConv.id, unatt), "added", g.providerId); }} className="size-7 grid place-items-center border border-border">{addP ? <Loader2 className="size-3 animate-spin" /> : <Plus className="size-3" />}</button>}
                          {att.length > 0 && <button disabled={pend} onClick={(e) => { e.stopPropagation(); runAction(remK, () => removeToolSchemas(activeConv.id, att.map((s) => s.name)), "removed", g.providerId); }} className="size-7 grid place-items-center border border-border text-[hsl(var(--destructive))]">{remP ? <Loader2 className="size-3 animate-spin" /> : <Trash2 className="size-3" />}</button>}
                        </div>
                      </div>
                      {!coll && <div className="divide-y divide-border">{g.tools.map((s) => {
                        const attached = attachedNames.has(s.name);
                        const addTK = `tool:add:${activeConv.id}:${s.name}`;
                        const remTK = `tool:remove:${activeConv.id}:${s.name}`;
                        const tp = pendingSet.has(addTK) || pendingSet.has(remTK) || pend;
                        return (
                          <div key={s.name} className="w-full min-h-8 pl-4 pr-2 py-1.5 flex items-center justify-between gap-2">
                            <span className="min-w-0 flex-1"><span className="block font-mono text-[11px] text-[hsl(var(--tool-call))] break-words">{s.providerToolName || s.name}</span><span className="block text-[10px] text-muted-foreground break-words">{s.description}</span></span>
                            {attached ? <button disabled={tp} onClick={() => runAction(remTK, () => removeToolSchema(activeConv.id, s.name), "removed", s.name)} className="size-7 grid place-items-center border border-border text-[hsl(var(--destructive))]">{pendingSet.has(remTK) ? <Loader2 className="size-3 animate-spin" /> : <Trash2 className="size-3" />}</button> : <button disabled={tp} onClick={() => runAction(addTK, () => addToolSchema(activeConv.id, s), "added", s.name)} className="size-7 grid place-items-center border border-border">{pendingSet.has(addTK) ? <Loader2 className="size-3 animate-spin" /> : <Plus className="size-3" />}</button>}
                          </div>
                        );
                      })}</div>}
                    </div>
                  );
                })}
                {unavailable.map((p) => (
                  <div key={p.providerId} className="border border-border bg-surface/20 px-2 py-2"><div className="font-mono text-[10px] uppercase text-muted-foreground">{p.displayName || p.providerId}</div><div className="font-mono text-[10px] uppercase text-[hsl(var(--destructive))]">unavailable</div>{p.error && <div className="text-[10px] text-muted-foreground break-words">{p.error}</div>}</div>
                ))}
              </div>
            ) : <div className="font-mono text-[11px] text-muted-foreground">no tool schemas</div>}
          </div>
        </Section>
      </div>
    </aside>
  );
}

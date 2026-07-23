import { useState, useMemo, useEffect, useRef } from "react";
import { useWindie } from "@/context/WindieContext";
import {
  ChevronRight,
  Trash2,
  ChevronDown,
  Plus,
  Loader2,
  X,
} from "lucide-react";
import { toast } from "sonner";
import ExtensionsPanel from "@/components/windie/ExtensionsPanel";

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

export default function InspectorPanel({ mode, onClose }) {
  const {
    activeConv,
    setSystemPrompt,
    setToolApprovalMode,
    toolSchemas,
    approvals,
    approveToolCall,
    denyToolCall,
    availableToolSchemas,
    availableToolsLoading,
    addToolSchema,
    addToolSchemas,
    removeToolSchema,
    removeToolSchemas,
    toolProviderStatuses,
  } = useWindie();

  const [sysDraft, setSysDraft] = useState(activeConv?.systemPrompt || "");
  const [pendingToolActionKeys, setPendingToolActionKeys] = useState([]);
  const [collapsedToolProviderIds, setCollapsedToolProviderIds] = useState(null);
  const [toolsView, setToolsView] = useState("attached");
  const pendingRef = useRef(new Set());
  const initRef = useRef(new Set());

  useEffect(() => setSysDraft(activeConv?.systemPrompt || ""), [activeConv?.id, activeConv?.systemPrompt]);

  useEffect(() => {
    if (mode !== "tools") setToolsView("attached");
  }, [mode]);

  const attachedNames = useMemo(() => new Set(toolSchemas.map((s) => s.name)), [toolSchemas]);
  const pendingSet = useMemo(() => new Set(pendingToolActionKeys), [pendingToolActionKeys]);

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

  // Provider groups start collapsed during the first render so opening the
  // tools overlay never briefly paints their expanded contents.
  const collapsedSet = useMemo(
    () => new Set(collapsedToolProviderIds ?? grouped.map((g) => g.providerId)),
    [collapsedToolProviderIds, grouped]
  );

  const toggle = (id) => setCollapsedToolProviderIds((c) => {
    const current = c ?? grouped.map((g) => g.providerId);
    return current.includes(id)
      ? current.filter((x) => x !== id)
      : [...current, id];
  });

  const saveSystemPrompt = async () => {
    await setSystemPrompt(activeConv.id, sysDraft);
    toast.message("system prompt updated");
    onClose();
  };

  useEffect(() => {
    const unseen = grouped.map((g) => g.providerId).filter((id) => !initRef.current.has(id));
    if (!unseen.length) return;
    unseen.forEach((id) => initRef.current.add(id));
    setCollapsedToolProviderIds((c) => [...new Set([...(c || []), ...unseen])]);
  }, [grouped]);

  useEffect(() => {
    const handleKey = (event) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [onClose]);

  if (!activeConv || !mode) return null;

  return (
    <div data-testid="windie-inspector-overlay" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }} className="absolute inset-0 z-40 bg-background/90 backdrop-blur-sm flex items-start justify-center px-6 pt-12 pb-6 overflow-y-auto windie-scroll" style={{ scrollbarGutter: "stable" }}>
      <div data-testid={`windie-${mode}-overlay`} className={`w-full border border-border bg-background shadow-lg flex flex-col ${mode === "system" ? "max-w-5xl min-h-[70vh]" : "max-w-4xl max-h-[calc(100vh-7rem)]"}`}>
        <div className="h-10 shrink-0 border-b border-border px-4 flex items-center justify-between">
          <span className="font-mono text-[11px] uppercase tracking-widest text-muted-foreground">{mode === "system" ? "system prompt" : "tools"}</span>
          <button type="button" data-testid="windie-overlay-close" onClick={onClose} aria-label="close overlay" className="p-1 text-muted-foreground hover:text-foreground hover:bg-surface-hover">
            <X className="size-3.5" strokeWidth={1.75} />
          </button>
        </div>
        {mode === "tools" && (
          <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border bg-surface/20 px-3">
            <button
              type="button"
              data-testid="tools-tab-attached"
              onClick={() => setToolsView("attached")}
              className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest transition-colors ${toolsView === "attached" ? "bg-foreground text-background" : "text-muted-foreground hover:bg-surface-hover hover:text-foreground"}`}
            >
              installed
            </button>
            <button
              type="button"
              data-testid="tools-tab-extensions"
              onClick={() => setToolsView("extensions")}
              className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest transition-colors ${toolsView === "extensions" ? "bg-foreground text-background" : "text-muted-foreground hover:bg-surface-hover hover:text-foreground"}`}
            >
              extensions
            </button>
            <span className="ml-auto font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
              {toolsView === "extensions" ? "local capabilities" : "conversation access"}
            </span>
          </div>
        )}
        <div className="flex-1 min-h-0 overflow-y-auto windie-scroll" style={{ scrollbarGutter: "stable" }}>
          {mode === "system" ? (
            <div className="min-h-[calc(70vh-2.5rem)] flex flex-col p-6 gap-4">
              <textarea
                data-testid="inspector-sysprompt-textarea"
                value={sysDraft}
                onChange={(e) => setSysDraft(e.target.value)}
                placeholder="Write the system prompt..."
                className="flex-1 min-h-[55vh] w-full resize-none bg-transparent border border-border p-4 font-mono text-sm leading-relaxed outline-none focus:border-foreground"
              />
              <div className="flex items-center justify-between">
                <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">conversation-wide</span>
                <button data-testid="inspector-sysprompt-commit" onClick={saveSystemPrompt} className="text-[10px] uppercase px-3 py-1.5 border border-foreground bg-foreground text-background font-mono">save</button>
              </div>
            </div>
          ) : toolsView === "extensions" ? (
            <ExtensionsPanel />
          ) : (
            <>

        <Section title={`approvals · ${approvals.length}`} testId="inspector-section-approvals">
          <div className="flex items-center gap-2 py-1 text-[11px]">
            <span className="text-muted-foreground font-mono uppercase tracking-widest w-24 shrink-0">tool access</span>
            <div className="grid grid-cols-2 border border-border">
              <button data-testid="tool-approval-mode-manual" onClick={() => { setToolApprovalMode(activeConv.id, "manual"); toast.message("tool access set", { description: "manual" }); }} className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest ${activeConv.toolApprovalMode === "manual" ? "bg-foreground text-background" : "text-muted-foreground hover:bg-surface-hover"}`}>manual</button>
              <button data-testid="tool-approval-mode-auto" onClick={() => { setToolApprovalMode(activeConv.id, "auto_approve_attached"); toast.message("tool access set", { description: "full access" }); }} className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest border-l border-border ${activeConv.toolApprovalMode === "auto_approve_attached" ? "bg-foreground text-background" : "text-muted-foreground hover:bg-surface-hover"}`}>full access</button>
            </div>
          </div>
          {approvals.length === 0 ? <div className="font-mono text-[11px] text-muted-foreground">no pending</div> : approvals.map((a) => (
            <div key={a.tool_call_id} className="border border-border mb-2">
              <div className="px-2 py-1 border-b border-border flex justify-between"><span className="font-mono text-[11px] text-[hsl(var(--tool-call))]">{a.tool_name}</span><span className="font-mono text-[9px] text-muted-foreground">session {a.session_id?.slice(0, 8)}</span></div>
              <div className="px-2 py-1.5 space-y-1">
                <div className="font-mono text-[10px] text-muted-foreground">{a.reason}</div>
                <pre className="font-mono text-[10px] bg-surface/60 border border-border p-2 overflow-auto max-h-32 whitespace-pre-wrap">{(() => { try { return JSON.stringify(JSON.parse(a.arguments), null, 2); } catch { return a.arguments; } })()}</pre>
                <div className="grid grid-cols-2 gap-1">
                  <button data-testid={`approval-approve-${a.tool_call_id}`} onClick={() => { approveToolCall(a.session_id, a.tool_call_id); toast.message("approved"); }} className="h-7 border border-foreground bg-foreground text-background font-mono text-[10px] uppercase">approve</button>
                  <button data-testid={`approval-deny-${a.tool_call_id}`} onClick={() => { denyToolCall(a.session_id, a.tool_call_id); toast.message("denied"); }} className="h-7 border border-border font-mono text-[10px] uppercase text-[hsl(var(--destructive))]">deny</button>
                </div>
              </div>
            </div>
          ))}
        </Section>

        <Section title={`tool schemas · ${availableToolsLoading ? "loading" : availableToolSchemas.length}`} testId="inspector-section-tools" resetKey={activeConv.id}>
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
            </>
          )}
        </div>
      </div>
    </div>
  );
}

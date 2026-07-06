import { useState, useMemo, useEffect } from "react";
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
} from "lucide-react";
import { toast } from "sonner";

function Section({ title, children, defaultOpen = true, right, testId }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="border-b border-border" data-testid={testId}>
      <button
        onClick={() => setOpen(!open)}
        className="w-full px-3 py-2 flex items-center justify-between hover:bg-surface/60 transition-colors"
      >
        <div className="flex items-center gap-1.5">
          {open ? (
            <ChevronDown className="size-3 text-muted-foreground" strokeWidth={1.75} />
          ) : (
            <ChevronRight className="size-3 text-muted-foreground" strokeWidth={1.75} />
          )}
          <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            {title}
          </span>
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
      <span className="text-muted-foreground font-mono uppercase tracking-widest w-24 shrink-0">
        {k}
      </span>
      <span className={mono ? "font-mono text-foreground break-all" : "text-foreground"}>{v}</span>
    </div>
  );
}

export default function InspectorPanel() {
  const {
    activeConv,
    selectedNodeId,
    setSelectedNodeId,
    setSystemPrompt,
    setActivePathToLeaf,
    forkFromMessage,
    truncateAfter,
    removeMessage,
    editMessage,
    toolSchemas,
    approvals,
    approveToolCall,
    denyToolCall,
    contextPreviewOpen,
    setContextPreviewOpen,
    modelOverride,
    availableToolSchemas,
    addToolSchema,
    removeToolSchema,
  } = useWindie();
  const [editingSys, setEditingSys] = useState(false);
  const [sysDraft, setSysDraft] = useState(activeConv?.systemPrompt || "");

  useEffect(() => {
    setSysDraft(activeConv?.systemPrompt || "");
  }, [activeConv?.id, activeConv?.systemPrompt]);

  const selectedNode = selectedNodeId ? activeConv?.nodes[selectedNodeId] : null;
  const onActivePath = selectedNode && activeConv.activePath.includes(selectedNodeId);
  const attachedToolNames = useMemo(
    () => new Set(toolSchemas.map((schema) => schema.name)),
    [toolSchemas]
  );

  const runtimeRequestPreview = useMemo(() => {
    if (!activeConv) return null;
    return {
      model: modelOverride || activeConv.model,
      system_prompt: activeConv.systemPrompt || null,
      messages: activeConv.modelContext || [],
      tools: toolSchemas,
      latest_compaction: activeConv.latestCompaction || null,
    };
  }, [activeConv, modelOverride, toolSchemas]);

  if (!activeConv) return null;

  return (
    <aside
      data-testid="windie-inspector"
      className="w-[340px] shrink-0 border-l border-border bg-background flex flex-col"
    >
      <div className="h-8 shrink-0 border-b border-border px-3 flex items-center font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        inspector
      </div>

      <div className="flex-1 min-h-0 overflow-y-auto windie-scroll">
        {/* Conversation Metadata */}
        <Section title="conversation" testId="inspector-section-conversation">
          <KV k="id" v={activeConv.id} />
          <KV k="model" v={activeConv.model} />
          <KV k="nodes" v={Object.keys(activeConv.nodes).length} />
          <KV k="branches" v={Object.values(activeConv.nodes).filter((n) => n.childrenIds.length > 1).length} />
          <KV k="updated" v={new Date(activeConv.updatedAt).toLocaleString()} />
          <div className="mt-1 flex flex-wrap gap-1">
            {(activeConv.tags || []).map((t) => (
              <span
                key={t}
                className="font-mono text-[10px] uppercase tracking-widest px-1.5 py-0.5 border border-border text-muted-foreground"
              >
                {t}
              </span>
            ))}
          </div>
        </Section>

        {/* Active Path */}
        <Section title="active path" testId="inspector-section-active-path">
          <div className="font-mono text-[10px] text-muted-foreground mb-1.5">
            {activeConv.activePath.length} nodes
          </div>
          <div className="space-y-0.5">
            {activeConv.activePath.map((id, i) => {
              const n = activeConv.nodes[id];
              if (!n) return null;
              const token = ROLE_TOKENS[n.message.role];
              const isSel = id === selectedNodeId;
              return (
                <button
                  key={id}
                  data-testid={`inspector-path-node-${id}`}
                  onClick={() => setSelectedNodeId(id)}
                  className={`w-full text-left flex items-center gap-2 px-1.5 py-1 border-l-2 font-mono text-[11px] ${
                    isSel
                      ? "bg-surface border-[hsl(var(--accent))]"
                      : "border-transparent hover:bg-surface/60"
                  }`}
                >
                  <span className="text-muted-foreground w-5 text-right">
                    {String(i).padStart(2, "0")}
                  </span>
                  <span className={`w-8 ${token.color}`}>[{token.label}]</span>
                  <span className="truncate flex-1 text-muted-foreground">
                    {(n.message.parts.find((p) => p.type === "text")?.text || "").slice(0, 40) ||
                      "(empty)"}
                  </span>
                </button>
              );
            })}
          </div>
        </Section>

        {/* System prompt */}
        <Section
          title="system prompt"
          testId="inspector-section-system-prompt"
          right={
            !editingSys && (
              <span
                data-testid="inspector-edit-sysprompt-icon"
                onClick={(e) => {
                  e.stopPropagation();
                  setSysDraft(activeConv.systemPrompt);
                  setEditingSys(true);
                }}
                className="p-1 hover:bg-surface-hover"
              >
                <Pencil className="size-3" strokeWidth={1.75} />
              </span>
            )
          }
        >
          {editingSys ? (
            <div className="space-y-2">
              <textarea
                data-testid="inspector-sysprompt-textarea"
                value={sysDraft}
                onChange={(e) => setSysDraft(e.target.value)}
                rows={5}
                className="w-full bg-transparent border border-foreground/60 p-2 font-mono text-[11px] outline-none resize-none leading-relaxed"
              />
              <div className="flex items-center gap-1">
                <button
                  data-testid="inspector-sysprompt-commit"
                  onClick={() => {
                    setSystemPrompt(activeConv.id, sysDraft);
                    setEditingSys(false);
                    toast.message("system prompt updated");
                  }}
                  className="text-[10px] uppercase tracking-widest px-2 py-1 border border-foreground bg-foreground text-background font-mono"
                >
                  commit
                </button>
                <button
                  onClick={() => setEditingSys(false)}
                  className="text-[10px] uppercase tracking-widest px-2 py-1 border border-border hover:bg-surface-hover font-mono"
                >
                  cancel
                </button>
              </div>
            </div>
          ) : (
            <div className="font-mono text-[11px] leading-relaxed text-muted-foreground whitespace-pre-wrap border-l-2 border-muted-foreground/40 pl-2 py-1">
              {activeConv.systemPrompt}
            </div>
          )}
        </Section>

        {/* Model request preview toggle */}
        <Section
          title="runtime request preview"
          testId="inspector-section-request-preview"
          right={
            <span
              data-testid="inspector-toggle-preview"
              onClick={(e) => {
                e.stopPropagation();
                setContextPreviewOpen(!contextPreviewOpen);
              }}
              className={`px-1.5 py-0.5 text-[10px] font-mono uppercase tracking-widest border ${
                contextPreviewOpen
                  ? "border-foreground bg-foreground text-background"
                  : "border-border"
              }`}
            >
              {contextPreviewOpen ? "expanded" : "collapsed"}
            </span>
          }
        >
          {contextPreviewOpen ? (
            <pre className="font-mono text-[10px] leading-relaxed text-muted-foreground bg-surface/60 border border-border p-2 overflow-x-auto max-h-64 overflow-y-auto whitespace-pre-wrap">
              {JSON.stringify(runtimeRequestPreview, null, 2)}
            </pre>
          ) : (
            <div className="font-mono text-[11px] text-muted-foreground">
              {runtimeRequestPreview?.messages.length} msgs · {runtimeRequestPreview?.tools.length}{" "}
              tools · api context
            </div>
          )}
        </Section>

        {/* Selected message */}
        <Section
          title={selectedNode ? `selected message · ${ROLE_TOKENS[selectedNode.message.role].label}` : "selected message"}
          testId="inspector-section-selected"
        >
          {!selectedNode ? (
            <div className="font-mono text-[11px] text-muted-foreground">no message selected</div>
          ) : (
            <>
              <KV k="node id" v={selectedNode.id} />
              <KV k="parent" v={selectedNode.parentId || "(root)"} />
              <KV k="children" v={selectedNode.childrenIds.length} />
              <KV
                k="on path"
                v={
                  onActivePath ? (
                    <span className="text-[hsl(var(--accent))]">yes</span>
                  ) : (
                    <span className="text-muted-foreground">no</span>
                  )
                }
              />
              {selectedNode.message.model && (
                <KV k="model" v={selectedNode.message.model} />
              )}
              {selectedNode.message.tokens && <KV k="tokens" v={selectedNode.message.tokens} />}

              <div className="mt-3 grid grid-cols-2 gap-1">
                <button
                  data-testid="inspector-action-fork"
                  onClick={() => {
                    forkFromMessage(activeConv.id, selectedNode.id);
                    toast.message("forked", { description: "new conversation created" });
                  }}
                  className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest"
                >
                  <GitBranch className="size-3" strokeWidth={1.75} /> fork
                </button>
                <button
                  data-testid="inspector-action-set-path"
                  onClick={() => {
                    setActivePathToLeaf(activeConv.id, selectedNode.id);
                    toast.message("active path set to selection");
                  }}
                  className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest"
                >
                  <Route className="size-3" strokeWidth={1.75} /> set path
                </button>
                <button
                  data-testid="inspector-action-truncate"
                  onClick={() => {
                    truncateAfter(activeConv.id, selectedNode.id);
                    toast.message("truncated", { description: "descendants deleted" });
                  }}
                  className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest"
                >
                  <Scissors className="size-3" strokeWidth={1.75} /> truncate
                </button>
                <button
                  data-testid="inspector-action-remove"
                  onClick={() => {
                    removeMessage(activeConv.id, selectedNode.id);
                    toast.message("message removed");
                  }}
                  className="h-8 flex items-center justify-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))]"
                >
                  <Trash2 className="size-3" strokeWidth={1.75} /> remove
                </button>
              </div>

              {/* Metadata lanes summary */}
              {selectedNode.message.metadata && (
                <div className="mt-3 space-y-1">
                  <div className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                    metadata lanes
                  </div>
                  <div className="grid grid-cols-2 gap-1 font-mono text-[10px]">
                    <MetaLane
                      k="tool_calls"
                      v={selectedNode.message.metadata.toolCalls?.length || 0}
                      color="var(--tool-call)"
                    />
                    <MetaLane
                      k="reasoning"
                      v={selectedNode.message.metadata.reasoning ? "•" : "—"}
                      color="var(--reasoning)"
                    />
                    <MetaLane
                      k="refusal"
                      v={selectedNode.message.metadata.refusal ? "•" : "—"}
                      color="var(--refusal)"
                    />
                    <MetaLane
                      k="annotations"
                      v={selectedNode.message.metadata.annotations?.length || 0}
                      color="var(--annotation)"
                    />
                    <MetaLane
                      k="audio"
                      v={selectedNode.message.metadata.audio ? "•" : "—"}
                      color="var(--audio)"
                    />
                  </div>
                </div>
              )}
            </>
          )}
        </Section>

        {/* Pending approvals */}
        <Section
          title={`approvals · ${approvals.length}`}
          testId="inspector-section-approvals"
          defaultOpen={approvals.length > 0}
        >
          {approvals.length === 0 ? (
            <div className="font-mono text-[11px] text-muted-foreground">
              no pending approvals
            </div>
          ) : (
            <div className="space-y-2">
              {approvals.map((approval) => (
                <div key={approval.tool_call_id} className="border border-border">
                  <div className="px-2 py-1 border-b border-border flex items-center justify-between">
                    <span className="font-mono text-[11px] text-[hsl(var(--tool-call))]">
                      {approval.tool_name}
                    </span>
                    <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                      {approval.tool_call_id.slice(0, 10)}
                    </span>
                  </div>
                  <div className="px-2 py-1.5 space-y-1">
                    <div className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                      {approval.reason}
                    </div>
                    <pre className="font-mono text-[10px] text-muted-foreground bg-surface/60 border border-border p-2 overflow-x-auto whitespace-pre-wrap max-h-32">
                      {formatArguments(approval.arguments)}
                    </pre>
                    <div className="grid grid-cols-2 gap-1 pt-1">
                      <button
                        data-testid={`approval-approve-${approval.tool_call_id}`}
                        onClick={() => {
                          approveToolCall(activeConv.id, approval.tool_call_id);
                          toast.message("tool approved");
                        }}
                        className="h-7 border border-foreground bg-foreground text-background font-mono text-[10px] uppercase tracking-widest hover:opacity-90"
                      >
                        approve
                      </button>
                      <button
                        data-testid={`approval-deny-${approval.tool_call_id}`}
                        onClick={() => {
                          denyToolCall(activeConv.id, approval.tool_call_id);
                          toast.message("tool denied");
                        }}
                        className="h-7 border border-border text-[hsl(var(--destructive))] font-mono text-[10px] uppercase tracking-widest hover:bg-surface-hover"
                      >
                        deny
                      </button>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </Section>

        {/* Tool schemas */}
        <Section
          title={`tool schemas · ${availableToolSchemas.length}`}
          testId="inspector-section-tools"
          defaultOpen={true}
        >
          <div className="space-y-2">
            {availableToolSchemas.length > 0 ? (
              <div className="space-y-1">
                {availableToolSchemas.map((schema) => {
                  const attached = attachedToolNames.has(schema.name);

                  return (
                    <div
                      key={schema.name}
                      className="w-full min-h-8 px-2 py-1.5 flex items-center justify-between gap-2 border border-border"
                    >
                      <span className="min-w-0">
                        <span className="block font-mono text-[11px] text-[hsl(var(--tool-call))]">
                          {schema.name}
                        </span>
                        <span className="block text-[10px] text-muted-foreground leading-snug">
                          {schema.description}
                        </span>
                      </span>
                      {attached ? (
                        <button
                          type="button"
                          data-testid={`tool-catalog-remove-${schema.name}`}
                          onClick={() => {
                            removeToolSchema(activeConv.id, schema.name);
                            toast.message("tool schema removed", { description: schema.name });
                          }}
                          className="size-7 grid place-items-center shrink-0 border border-border text-[hsl(var(--destructive))] hover:bg-surface-hover"
                          aria-label={`Remove ${schema.name}`}
                        >
                          <Trash2 className="size-3" strokeWidth={1.75} />
                        </button>
                      ) : (
                        <button
                          type="button"
                          data-testid={`tool-catalog-add-${schema.name}`}
                          onClick={() => {
                            addToolSchema(activeConv.id, schema);
                            toast.message("tool schema added", { description: schema.name });
                          }}
                          className="size-7 grid place-items-center shrink-0 border border-border hover:bg-surface-hover"
                          aria-label={`Add ${schema.name}`}
                        >
                          <Plus className="size-3 text-muted-foreground" strokeWidth={1.75} />
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="font-mono text-[11px] text-muted-foreground">
                no tool schemas
              </div>
            )}
          </div>
        </Section>
      </div>
    </aside>
  );
}

function formatArguments(value) {
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

function MetaLane({ k, v, color }) {
  return (
    <div
      className="border-l-2 pl-1.5 py-0.5"
      style={{ borderColor: `hsl(${color})` }}
    >
      <div
        className="uppercase tracking-widest text-[9px]"
        style={{ color: `hsl(${color})` }}
      >
        {k}
      </div>
      <div className="text-foreground">{v}</div>
    </div>
  );
}

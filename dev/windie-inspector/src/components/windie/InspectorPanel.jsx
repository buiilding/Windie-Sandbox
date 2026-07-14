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
    setToolApprovalMode,
    setActivePathToLeaf,
    forkFromMessage,
    truncateAfter,
    removeMessage,
    editMessage,
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
  } = useWindie();
  const [editingSys, setEditingSys] = useState(false);
  const [sysDraft, setSysDraft] = useState(activeConv?.systemPrompt || "");
  const [pendingToolActionKeys, setPendingToolActionKeys] = useState([]);
  const [collapsedToolProviderIds, setCollapsedToolProviderIds] = useState([]);
  const pendingToolActionKeysRef = useRef(new Set());
  const initializedToolProviderIdsRef = useRef(new Set());

  useEffect(() => {
    setSysDraft(activeConv?.systemPrompt || "");
  }, [activeConv?.id, activeConv?.systemPrompt]);

  const selectedNode = selectedNodeId ? activeConv?.nodes[selectedNodeId] : null;
  const onActivePath = selectedNode && activeConv.activePath.includes(selectedNodeId);
  const attachedToolNames = useMemo(
    () => new Set(toolSchemas.map((schema) => schema.name)),
    [toolSchemas]
  );
  const pendingToolActions = useMemo(
    () => new Set(pendingToolActionKeys),
    [pendingToolActionKeys]
  );
  const collapsedToolProviders = useMemo(
    () => new Set(collapsedToolProviderIds),
    [collapsedToolProviderIds]
  );
  const toggleToolProvider = (providerId) => {
    setCollapsedToolProviderIds((current) =>
      current.includes(providerId)
        ? current.filter((id) => id !== providerId)
        : [...current, providerId]
    );
  };
  const setToolActionPending = (key, pending) => {
    const next = new Set(pendingToolActionKeysRef.current);
    if (pending) next.add(key);
    else next.delete(key);
    pendingToolActionKeysRef.current = next;
    setPendingToolActionKeys([...next]);
  };
  const runToolAction = async (key, action, successMessage, description) => {
    if (pendingToolActionKeysRef.current.has(key)) return;
    setToolActionPending(key, true);
    try {
      await action();
      toast.message(successMessage, description ? { description } : undefined);
    } finally {
      setToolActionPending(key, false);
    }
  };
  const groupedToolSchemas = useMemo(
    () => groupToolSchemasByProvider(availableToolSchemas),
    [availableToolSchemas]
  );
  const unavailableToolProviders = useMemo(
    () => (toolProviderStatuses || []).filter((provider) => !provider.available),
    [toolProviderStatuses]
  );

  useEffect(() => {
    const unseenProviderIds = groupedToolSchemas
      .map((group) => group.providerId)
      .filter((providerId) => !initializedToolProviderIdsRef.current.has(providerId));
    if (unseenProviderIds.length === 0) return;

    for (const providerId of unseenProviderIds) {
      initializedToolProviderIdsRef.current.add(providerId);
    }
    setCollapsedToolProviderIds((current) => [
      ...new Set([...current, ...unseenProviderIds]),
    ]);
  }, [groupedToolSchemas]);

  if (!activeConv) return null;

  return (
    <aside
      data-testid="windie-inspector"
      className="w-[340px] shrink-0 border-l border-border bg-background flex flex-col"
    >
      <div className="h-8 shrink-0 border-b border-border px-3 flex items-center font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        inspector
      </div>

      <div
        className="flex-1 min-h-0 overflow-y-auto overflow-x-hidden windie-scroll"
        style={{ scrollbarGutter: "stable" }}
      >
        {/* Conversation Metadata */}
        <Section title="conversation" testId="inspector-section-conversation">
          <KV k="id" v={activeConv.id} />
          <KV k="model" v={activeConv.model} />
          <KV k="nodes" v={Object.keys(activeConv.nodes).length} />
          <KV k="branches" v={Object.values(activeConv.nodes).filter((n) => n.childrenIds.length > 1).length} />
          <KV k="updated" v={new Date(activeConv.updatedAt).toLocaleString()} />
          <div className="flex items-center gap-2 py-1 text-[11px]">
            <span className="text-muted-foreground font-mono uppercase tracking-widest w-24 shrink-0">
              tool access
            </span>
            <div className="grid grid-cols-2 border border-border">
              <button
                type="button"
                data-testid="tool-approval-mode-manual"
                onClick={() => {
                  setToolApprovalMode(activeConv.id, "manual");
                  toast.message("tool access set", { description: "manual" });
                }}
                className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest ${
                  activeConv.toolApprovalMode === "manual"
                    ? "bg-foreground text-background"
                    : "text-muted-foreground hover:bg-surface-hover"
                }`}
              >
                manual
              </button>
              <button
                type="button"
                data-testid="tool-approval-mode-auto"
                onClick={() => {
                  setToolApprovalMode(activeConv.id, "auto_approve_attached");
                  toast.message("tool access set", { description: "full access" });
                }}
                className={`h-7 px-2 font-mono text-[10px] uppercase tracking-widest border-l border-border ${
                  activeConv.toolApprovalMode === "auto_approve_attached"
                    ? "bg-foreground text-background"
                    : "text-muted-foreground hover:bg-surface-hover"
                }`}
              >
                full access
              </button>
            </div>
          </div>
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
        <Section title="active path" testId="inspector-section-active-path" defaultOpen={false}>
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
          defaultOpen={false}
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

            </>
          )}
        </Section>

        {/* Pending approvals */}
        <Section
          title={`approvals · ${approvals.length}`}
          testId="inspector-section-approvals"
          defaultOpen={true}
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
                      run {approval.run_id?.slice(0, 8)}
                    </span>
                  </div>
                  <div className="px-2 py-1.5 space-y-1">
                    <div className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                      {approval.reason}
                    </div>
                    <pre className="font-mono text-[10px] text-muted-foreground bg-surface/60 border border-border p-2 overflow-x-auto whitespace-pre-wrap max-h-32">
                      {formatArguments(approval.arguments)}
                    </pre>
                    <div className="grid grid-cols-3 gap-1 pt-1">
                      <button
                        data-testid={`approval-view-${approval.tool_call_id}`}
                        onClick={() => {
                          setSelectedNodeId(approval.assistant_message_id);
                          toast.message("approval path selected");
                        }}
                        className="h-7 border border-border font-mono text-[10px] uppercase tracking-widest hover:bg-surface-hover"
                      >
                        view path
                      </button>
                      <button
                        data-testid={`approval-approve-${approval.tool_call_id}`}
                        onClick={() => {
                          approveToolCall(approval.run_id, approval.tool_call_id);
                          toast.message("tool approved");
                        }}
                        className="h-7 border border-foreground bg-foreground text-background font-mono text-[10px] uppercase tracking-widest hover:opacity-90"
                      >
                        approve
                      </button>
                      <button
                        data-testid={`approval-deny-${approval.tool_call_id}`}
                        onClick={() => {
                          denyToolCall(approval.run_id, approval.tool_call_id);
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
          resetKey={activeConv.id}
        >
          <div className="space-y-2">
            {availableToolSchemas.length > 0 || unavailableToolProviders.length > 0 ? (
              <div className="space-y-2">
                {groupedToolSchemas.map((group) => {
                  const unattachedTools = group.tools.filter(
                    (schema) => !attachedToolNames.has(schema.name)
                  );
                  const attachedTools = group.tools.filter((schema) =>
                    attachedToolNames.has(schema.name)
                  );
                  const addProviderKey = `provider:add:${activeConv.id}:${group.providerId}`;
                  const removeProviderKey = `provider:remove:${activeConv.id}:${group.providerId}`;
                  const addProviderPending = pendingToolActions.has(addProviderKey);
                  const removeProviderPending = pendingToolActions.has(removeProviderKey);
                  const providerPending = addProviderPending || removeProviderPending;
                  const providerCollapsed = collapsedToolProviders.has(group.providerId);

                  return (
                    <div key={group.providerId} className="border border-border">
                      <div
                        role="button"
                        tabIndex={0}
                        onClick={() => toggleToolProvider(group.providerId)}
                        onKeyDown={(event) => {
                          if (event.key === "Enter" || event.key === " ") {
                            event.preventDefault();
                            toggleToolProvider(group.providerId);
                          }
                        }}
                        className={`w-full min-h-8 px-2 py-1.5 flex items-center justify-between gap-2 bg-surface/40 hover:bg-surface-hover ${
                          providerCollapsed ? "" : "border-b border-border"
                        }`}
                        aria-expanded={!providerCollapsed}
                      >
                        <div className="min-w-0 text-left">
                          <div className="font-mono text-[10px] uppercase tracking-widest text-foreground">
                            {providerLabel(group.providerId)}
                          </div>
                          <div className="font-mono text-[10px] text-muted-foreground">
                            {group.tools.length} tool{group.tools.length === 1 ? "" : "s"}
                          </div>
                        </div>
                        <div className="flex shrink-0 items-center gap-1">
                          {unattachedTools.length > 0 && (
                            <button
                              type="button"
                              data-testid={`tool-provider-add-${group.providerId}`}
                              disabled={providerPending}
                              onClick={(event) => {
                                event.stopPropagation();
                                runToolAction(
                                  addProviderKey,
                                  () => addToolSchemas(activeConv.id, unattachedTools),
                                  "tool provider added",
                                  providerLabel(group.providerId)
                                );
                              }}
                              className="size-7 grid place-items-center border border-border hover:bg-surface-hover disabled:cursor-wait disabled:opacity-50"
                              aria-label={`Add ${providerLabel(group.providerId)} tools`}
                            >
                              {addProviderPending ? (
                                <Loader2 className="size-3 text-muted-foreground animate-spin" strokeWidth={1.75} />
                              ) : (
                                <Plus className="size-3 text-muted-foreground" strokeWidth={1.75} />
                              )}
                            </button>
                          )}
                          {attachedTools.length > 0 && (
                            <button
                              type="button"
                              data-testid={`tool-provider-remove-${group.providerId}`}
                              disabled={providerPending}
                              onClick={(event) => {
                                event.stopPropagation();
                                runToolAction(
                                  removeProviderKey,
                                  () =>
                                    removeToolSchemas(
                                      activeConv.id,
                                      attachedTools.map((schema) => schema.name)
                                    ),
                                  "tool provider removed",
                                  providerLabel(group.providerId)
                                );
                              }}
                              className="size-7 grid place-items-center border border-border text-[hsl(var(--destructive))] hover:bg-surface-hover disabled:cursor-wait disabled:opacity-50"
                              aria-label={`Remove ${providerLabel(group.providerId)} tools`}
                            >
                              {removeProviderPending ? (
                                <Loader2 className="size-3 animate-spin" strokeWidth={1.75} />
                              ) : (
                                <Trash2 className="size-3" strokeWidth={1.75} />
                              )}
                            </button>
                          )}
                        </div>
                      </div>

                      {!providerCollapsed && <div className="divide-y divide-border">
                        {group.tools.map((schema) => {
                          const attached = attachedToolNames.has(schema.name);
                          const displayName = schema.providerToolName || schema.name;
                          const addToolKey = `tool:add:${activeConv.id}:${schema.name}`;
                          const removeToolKey = `tool:remove:${activeConv.id}:${schema.name}`;
                          const addToolPending = pendingToolActions.has(addToolKey);
                          const removeToolPending = pendingToolActions.has(removeToolKey);
                          const toolPending = addToolPending || removeToolPending || providerPending;

                          return (
                            <div
                              key={schema.name}
                              className="w-full min-h-8 pl-4 pr-2 py-1.5 flex items-center justify-between gap-2"
                            >
                              <span className="min-w-0 flex-1 overflow-hidden">
                                <span className="block font-mono text-[11px] text-[hsl(var(--tool-call))] break-words">
                                  {displayName}
                                </span>
                                <span className="block text-[10px] text-muted-foreground leading-snug break-words">
                                  {schema.description}
                                </span>
                              </span>
                              {attached ? (
                                <button
                                  type="button"
                                  data-testid={`tool-catalog-remove-${schema.name}`}
                                  disabled={toolPending}
                                  onClick={() =>
                                    runToolAction(
                                      removeToolKey,
                                      () => removeToolSchema(activeConv.id, schema.name),
                                      "tool schema removed",
                                      schema.name
                                    )
                                  }
                                  className="size-7 grid place-items-center shrink-0 border border-border text-[hsl(var(--destructive))] hover:bg-surface-hover disabled:cursor-wait disabled:opacity-50"
                                  aria-label={`Remove ${schema.name}`}
                                >
                                  {removeToolPending ? (
                                    <Loader2 className="size-3 animate-spin" strokeWidth={1.75} />
                                  ) : (
                                    <Trash2 className="size-3" strokeWidth={1.75} />
                                  )}
                                </button>
                              ) : (
                                <button
                                  type="button"
                                  data-testid={`tool-catalog-add-${schema.name}`}
                                  disabled={toolPending}
                                  onClick={() =>
                                    runToolAction(
                                      addToolKey,
                                      () => addToolSchema(activeConv.id, schema),
                                      "tool schema added",
                                      schema.name
                                    )
                                  }
                                  className="size-7 grid place-items-center shrink-0 border border-border hover:bg-surface-hover disabled:cursor-wait disabled:opacity-50"
                                  aria-label={`Add ${schema.name}`}
                                >
                                  {addToolPending ? (
                                    <Loader2 className="size-3 text-muted-foreground animate-spin" strokeWidth={1.75} />
                                  ) : (
                                    <Plus className="size-3 text-muted-foreground" strokeWidth={1.75} />
                                  )}
                                </button>
                              )}
                            </div>
                          );
                        })}
                      </div>}
                    </div>
                  );
                })}
                {unavailableToolProviders.map((provider) => (
                  <div
                    key={provider.providerId}
                    className="border border-border bg-surface/20 px-2 py-2"
                  >
                    <div className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                      {provider.displayName || providerLabel(provider.providerId)}
                    </div>
                    <div className="mt-1 font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))]">
                      unavailable
                    </div>
                    {provider.error && (
                      <div className="mt-1 text-[10px] text-muted-foreground leading-snug break-words">
                        {provider.error}
                      </div>
                    )}
                  </div>
                ))}
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

function providerLabel(providerId) {
  if (providerId === "windie") return "Windie";
  if (providerId === "cua-driver") return "CUA Driver";
  if (providerId === "desktop-commander") return "Desktop Commander";
  if (providerId === "blender-mcp") return "Blender MCP";
  if (providerId === "brightdata") return "Bright Data";
  return providerId || "Unknown Provider";
}

function groupToolSchemasByProvider(toolSchemas) {
  const groups = [];
  const groupByProvider = new Map();

  for (const schema of toolSchemas) {
    const providerId = schema.providerId || "unknown";
    let group = groupByProvider.get(providerId);
    if (!group) {
      group = { providerId, tools: [] };
      groupByProvider.set(providerId, group);
      groups.push(group);
    }
    group.tools.push(schema);
  }

  return groups;
}

function formatArguments(value) {
  try {
    return JSON.stringify(JSON.parse(value), null, 2);
  } catch {
    return value;
  }
}

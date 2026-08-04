import { useMemo, useState } from "react";
import { useWindie } from "@/context/WindieContext";

function formatArguments(argumentsValue) {
  try {
    return JSON.stringify(JSON.parse(argumentsValue), null, 2);
  } catch {
    return argumentsValue || "{}";
  }
}

/**
 * Renders session-owned tool approvals at the point where the user is about
 * to provide the next runtime input. The backend remains authoritative: this
 * component only displays pending records and forwards the user's decision.
 */
export default function ToolApprovalPrompt() {
  const { approvals, selectedSession, approveToolCall, denyToolCall } = useWindie();
  const [pendingToolCallIds, setPendingToolCallIds] = useState(() => new Set());

  const pendingApprovals = useMemo(
    () => approvals.filter((approval) => approval.session_id === selectedSession?.id),
    [approvals, selectedSession?.id]
  );

  if (!selectedSession || pendingApprovals.length === 0) return null;

  const runDecision = async (approval, decision) => {
    if (pendingToolCallIds.has(approval.tool_call_id)) return;
    setPendingToolCallIds((current) => new Set(current).add(approval.tool_call_id));
    try {
      if (decision === "approve") {
        await approveToolCall(approval.session_id, approval.tool_call_id);
      } else {
        await denyToolCall(approval.session_id, approval.tool_call_id);
      }
    } finally {
      setPendingToolCallIds((current) => {
        const next = new Set(current);
        next.delete(approval.tool_call_id);
        return next;
      });
    }
  };

  return (
    <section
      data-testid="tool-approval-prompt"
      aria-label="tool approval required"
      className="border-t border-[hsl(var(--tool-call))]/40 bg-[hsl(var(--tool-call))]/5 px-6 py-3"
    >
      <div className="flex items-center justify-between gap-3">
        <span className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--tool-call))]">
          permission required
        </span>
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          {pendingApprovals.length} pending
        </span>
      </div>

      <div className="mt-2 space-y-2">
        {pendingApprovals.map((approval) => {
          const pending = pendingToolCallIds.has(approval.tool_call_id);
          return (
            <div
              key={approval.tool_call_id}
              className="border border-[hsl(var(--tool-call))]/35 bg-background"
            >
              <div className="flex items-center justify-between gap-3 border-b border-border px-2.5 py-1.5">
                <span className="min-w-0 break-words font-mono text-[11px] text-[hsl(var(--tool-call))]">
                  {approval.tool_name}
                </span>
                <span className="shrink-0 font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                  session {approval.session_id?.slice(0, 8)}
                </span>
              </div>

              <div className="space-y-2 px-2.5 py-2">
                {approval.reason ? (
                  <p className="font-mono text-[10px] leading-relaxed text-muted-foreground">
                    {approval.reason}
                  </p>
                ) : null}
                <pre className="max-h-32 overflow-auto whitespace-pre-wrap border border-border bg-surface/60 p-2 font-mono text-[10px]">
                  {formatArguments(approval.arguments)}
                </pre>
                <div className="grid grid-cols-2 gap-1.5">
                  <button
                    type="button"
                    data-testid={`approval-prompt-approve-${approval.tool_call_id}`}
                    disabled={pending}
                    onClick={() => runDecision(approval, "approve")}
                    className="h-8 border border-foreground bg-foreground font-mono text-[10px] uppercase tracking-widest text-background disabled:cursor-wait disabled:opacity-60"
                  >
                    approve
                  </button>
                  <button
                    type="button"
                    data-testid={`approval-prompt-deny-${approval.tool_call_id}`}
                    disabled={pending}
                    onClick={() => runDecision(approval, "deny")}
                    className="h-8 border border-border font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))] disabled:cursor-wait disabled:opacity-60"
                  >
                    deny
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useWindie } from "@/context/WindieContext";
import { fetchImageAsset } from "@/lib/windieApi";
import { ROLE_TOKENS } from "@/lib/mockData";
import {
  GitBranch,
  Scissors,
  Trash2,
  Pencil,
  Copy,
  MoreHorizontal,
  Wrench,
  Check,
  X,
  Square,
  Image as ImageIcon,
  Target,
  ChevronDown,
} from "lucide-react";
import { toast } from "sonner";

const USER_MESSAGE_PREVIEW_LENGTH = 500;

function RoleBadge({ role }) {
  const token = ROLE_TOKENS[role] || ROLE_TOKENS.user;
  return (
    <span
      className={`font-mono text-[10px] font-bold tracking-widest ${token.color}`}
      data-testid={`msg-role-badge-${role}`}
    >
      [{token.label}]
    </span>
  );
}

function ReasoningLane({ reasoning, placeholder = false }) {
  const [open, setOpen] = useState(false);
  if (!reasoning && !placeholder) return null;
  const canExpand = Boolean(reasoning);
  return (
    <div className="mb-3">
      <button
        type="button"
        aria-expanded={canExpand && open}
        onClick={() => {
          if (canExpand) setOpen((value) => !value);
        }}
        className="group flex items-center gap-1 font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--reasoning))] hover:text-foreground transition-colors"
      >
        <span>thinking</span>
        <ChevronDown className="size-3 opacity-0 group-hover:opacity-100" strokeWidth={1.75} />
      </button>
      <div className={`windie-reasoning-content ${canExpand && open ? "open" : ""}`}>
        <div className="windie-reasoning-inner">
          {reasoning ? (
            <div className="mt-1 border-l-2 border-[hsl(var(--reasoning))] pl-2 py-1 bg-[hsl(var(--reasoning))]/5">
              <div className="text-xs text-muted-foreground italic leading-relaxed">
                {reasoning}
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function PendingThinkingLane({ pendingAssistant }) {
  if (!pendingAssistant.reasoning && pendingAssistant.text) return null;
  return <ReasoningLane reasoning={pendingAssistant.reasoning} placeholder />;
}

function MetadataLanes({ metadata }) {
  if (!metadata) return null;
  const lanes = [];
  if (metadata.toolCalls?.length) {
    lanes.push(
      <div
        key="tc"
        className="border-l-2 border-[hsl(var(--tool-call))] pl-2 py-1 bg-[hsl(var(--tool-call))]/5"
      >
        <div className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--tool-call))]">
          tool_calls · {metadata.toolCalls.length}
        </div>
        {metadata.toolCalls.map((tc) => (
          <div key={tc.id} className="mt-1 font-mono text-[11px]">
            <span className="text-[hsl(var(--tool-call))]">{tc.name}</span>
            <span className="text-muted-foreground">
              (
              {Object.entries(tc.arguments)
                .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
                .join(", ")}
              )
            </span>
          </div>
        ))}
      </div>
    );
  }
  if (metadata.refusal) {
    lanes.push(
      <div
        key="rf"
        className="border-l-2 border-[hsl(var(--refusal))] pl-2 py-1 bg-[hsl(var(--refusal))]/5"
      >
        <div className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--refusal))]">
          refusal · {metadata.refusal.category}
        </div>
        <div className="mt-0.5 text-xs text-muted-foreground leading-relaxed">
          {metadata.refusal.reason}
        </div>
      </div>
    );
  }
  if (metadata.annotations?.length) {
    lanes.push(
      <div
        key="an"
        className="border-l-2 border-[hsl(var(--annotation))] pl-2 py-1 bg-[hsl(var(--annotation))]/5"
      >
        <div className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--annotation))]">
          annotations · {metadata.annotations.length}
        </div>
        <ul className="mt-0.5 space-y-0.5">
          {metadata.annotations.map((a, i) => (
            <li key={i} className="text-xs">
              <span className="font-mono text-[hsl(var(--annotation))]">{a.label}</span>
              <span className="text-muted-foreground"> — {a.note}</span>
            </li>
          ))}
        </ul>
      </div>
    );
  }
  if (metadata.audio) {
    lanes.push(
      <div
        key="au"
        className="border-l-2 border-[hsl(var(--audio))] pl-2 py-1 bg-[hsl(var(--audio))]/5"
      >
        <div className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--audio))]">
          audio · {metadata.audio.source}
        </div>
        <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">
          {metadata.audio.durationSec}s · {metadata.audio.speakers} spk ·{" "}
          {metadata.audio.transcriptTokens}tok
        </div>
      </div>
    );
  }
  if (!lanes.length) return null;
  return <div className="mt-3 space-y-1.5">{lanes}</div>;
}

function PendingMetadataLanes({ pendingAssistant }) {
  const toolCalls = Object.entries(pendingAssistant.toolCalls || {}).map(
    ([index, call]) => ({ index, ...call })
  );
  const lanes = [];

  if (toolCalls.length) {
    lanes.push(
      <div
        key="tc"
        className="border-l-2 border-[hsl(var(--tool-call))] pl-2 py-1 bg-[hsl(var(--tool-call))]/5"
      >
        <div className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--tool-call))]">
          tool_calls · {toolCalls.length}
        </div>
        {toolCalls.map((tc) => (
          <div key={tc.id || tc.index} className="mt-1 font-mono text-[11px]">
            <span className="text-[hsl(var(--tool-call))]">
              {tc.name || "function_call"}
            </span>
            <span className="text-muted-foreground">
              {tc.id ? ` · ${tc.id}` : ""}
            </span>
            {tc.argumentsText ? (
              <pre className="mt-1 whitespace-pre-wrap break-words text-[11px] text-muted-foreground">
                {tc.argumentsText}
              </pre>
            ) : null}
          </div>
        ))}
      </div>
    );
  }

  if (!lanes.length) return null;
  return <div className="mt-3 space-y-1.5">{lanes}</div>;
}

function MessageImagePreview({ image, testId }) {
  const [objectUrl, setObjectUrl] = useState(image.url || "");
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (image.url) {
      setObjectUrl(image.url);
      setFailed(false);
      return undefined;
    }
    if (!image.assetId || !image.conversationId) {
      setObjectUrl("");
      setFailed(false);
      return undefined;
    }

    let active = true;
    let nextObjectUrl = "";
    setObjectUrl("");
    setFailed(false);

    fetchImageAsset(image.conversationId, image.assetId)
      .then((blob) => {
        if (!active) return;
        nextObjectUrl = URL.createObjectURL(blob);
        setObjectUrl(nextObjectUrl);
      })
      .catch(() => {
        if (active) setFailed(true);
      });

    return () => {
      active = false;
      if (nextObjectUrl) URL.revokeObjectURL(nextObjectUrl);
    };
  }, [image.url, image.assetId, image.conversationId]);

  return (
    <div
      className="border border-border p-1 max-w-[280px]"
      data-testid={testId}
    >
      {objectUrl && !failed ? (
        <img
          src={objectUrl}
          alt={image.alt || "attachment"}
          className="max-h-40 object-cover"
        />
      ) : (
        <div className="h-20 w-48 flex items-center justify-center bg-surface text-muted-foreground">
          <ImageIcon className="size-5" />
        </div>
      )}
      <div className="mt-1 flex items-center gap-1.5 font-mono text-[10px] text-muted-foreground">
        <ImageIcon className="size-3" />
        <span className="truncate">{image.alt || "attachment"}</span>
      </div>
    </div>
  );
}

function MessageMarkdown({ text, isStreaming }) {
  return (
    <div className="windie-markdown text-sm leading-relaxed font-sans">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        skipHtml
        components={{
          a: ({ node: _node, ...props }) => (
            <a
              {...props}
              target="_blank"
              rel="noreferrer"
              onClick={(event) => event.stopPropagation()}
            />
          ),
        }}
      >
        {text || ""}
      </ReactMarkdown>
    </div>
  );
}

/**
 * Live assistant preview row rendered from ephemeral `assistant_delta` text.
 *
 * This row has no store-backed node: it exists only while a query streams and
 * is replaced by the durable persisted message once `assistant_message_saved`
 * arrives. It intentionally omits selection, edit, fork, and tree actions
 * because there is no message id to act on yet. `sessionId` labels which
 * session is streaming and wires the per-session stop button.
 */
export function PendingAssistantRow({ pendingAssistant, index, sessionId, onStop }) {
  return (
    <div
      data-testid={`msg-row-pending-assistant${sessionId ? `-${sessionId.slice(0, 8)}` : ""}`}
      className="windie-message-assistant relative border-b border-border pt-3.5 pb-12 px-6"
    >
      <div className="absolute right-6 top-3.5 z-10 flex items-center gap-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        <span className="text-foreground/70">streaming</span>
        {sessionId && <span className="text-muted-foreground/70">· session {sessionId.slice(0, 8)}</span>}
        {onStop && (
          <button
            type="button"
            data-testid={`pending-stop-${sessionId?.slice(0, 8) || "session"}`}
            onClick={onStop}
            className="inline-flex items-center gap-1 border border-border px-1.5 py-0.5 text-[9px] uppercase tracking-widest text-muted-foreground hover:bg-surface-hover hover:text-foreground"
          >
            <Square className="size-2 fill-current" />
            stop
          </button>
        )}
      </div>
      <div className="flex items-baseline gap-3">
        <div className="w-16 shrink-0 pt-0.5">
          <RoleBadge role="assistant" />
        </div>
        <div className="flex-1 min-w-0 pt-2">
          <PendingThinkingLane pendingAssistant={pendingAssistant} />
          <MessageMarkdown text={pendingAssistant.text} isStreaming />
          <PendingMetadataLanes pendingAssistant={pendingAssistant} />
        </div>
      </div>
    </div>
  );
}

export default function MessageRow({ node, index, isLast }) {
  const {
    selectedNodeId,
    setSelectedNodeId,
    setPathHead,
    activeConv,
    forkFromMessage,
    truncateAfter,
    removeMessage,
    editMessage,
  } = useWindie();
  const [editing, setEditing] = useState(false);
  const [userMessageExpanded, setUserMessageExpanded] = useState(false);
  const [draft, setDraft] = useState(() => node.message.parts.find((p) => p.type === "text")?.text || "");

  const isSelected = selectedNodeId === node.id;
  const role = node.message.role;
  const isSystem = role === "system";
  const isUserMessage = role === "user";
  const messageTint = role === "user"
    ? "windie-message-user"
    : role === "assistant"
      ? "windie-message-assistant"
      : "";
  const rowSurface = isSelected
    ? "windie-message-selected"
    : messageTint
      ? ""
      : "hover:bg-surface/40";
  const isStreaming = node.message.streaming;
  const textPart = node.message.parts.find((p) => p.type === "text");
  const imageParts = node.message.parts.filter((p) => p.type === "image");
  const siblings = node.parentId
    ? activeConv.nodes[node.parentId]?.childrenIds || []
    : [node.id];
  const hasSiblings = siblings.length > 1;
  const userText = textPart?.text || "";
  const isLongUserMessage = isUserMessage && userText.length > USER_MESSAGE_PREVIEW_LENGTH;
  const visibleText = isLongUserMessage && !userMessageExpanded
    ? `${userText.slice(0, USER_MESSAGE_PREVIEW_LENGTH).trimEnd()}…`
    : userText;

  const commitEdit = () => {
    editMessage(activeConv.id, node.id, draft);
    setEditing(false);
    toast.message("message edited", { description: "created sibling on new path" });
  };

  const copyMessage = async () => {
    try {
      await navigator.clipboard.writeText(textPart?.text || "");
      toast.message("message copied");
    } catch {
      toast.error("could not copy message");
    }
  };

  return (
    <div
      data-testid={`msg-row-${node.id}`}
      onClick={() => setSelectedNodeId(node.id)}
      className={`group relative border-b border-border pt-3.5 pb-12 px-6 transition-colors cursor-pointer ${messageTint} ${rowSurface}`}
    >
      {isSelected && (
        <div className="absolute left-0 top-0 bottom-0 w-[3px] bg-[hsl(var(--accent))]" />
      )}
      {node.message.timestamp && (
        <span className="absolute right-6 top-3.5 font-mono text-[10px] uppercase tracking-widest text-muted-foreground/60">
          {new Date(node.message.timestamp).toLocaleTimeString([], {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </span>
      )}
      <div className="flex items-baseline gap-3">
        <div className="w-16 shrink-0 pt-0.5">
          <RoleBadge role={role} />
          {hasSiblings && (
            <div className="mt-1 font-mono text-[10px] text-[hsl(var(--accent))]">
              {siblings.indexOf(node.id) + 1}/{siblings.length}
            </div>
          )}
        </div>

        <div className="flex-1 min-w-0 pt-2">
          {(node.message.tokens || node.message.metadata?.toolCallId) && (
            <div className="flex items-center gap-2 mb-1.5 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            {node.message.tokens && <span>· {node.message.tokens}tok</span>}
            {node.message.metadata?.toolCallId && (
              <span className="text-[hsl(var(--tool-call))]">
                · call {node.message.metadata.toolCallId}
              </span>
            )}
            </div>
          )}

          {editing ? (
            <div className="space-y-2">
              <textarea
                data-testid={`msg-edit-textarea-${node.id}`}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onClick={(e) => e.stopPropagation()}
                autoFocus
                rows={Math.min(12, Math.max(3, draft.split("\n").length + 1))}
                className="w-full min-h-[50vh] max-h-[70vh] overflow-y-auto bg-transparent border border-foreground/60 p-3 font-mono text-xs leading-relaxed outline-none resize-y"
              />
              <div className="flex items-center gap-2">
                <button
                  data-testid={`msg-edit-commit-${node.id}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    commitEdit();
                  }}
                  className="text-[11px] font-mono uppercase tracking-widest px-2 py-1 border border-foreground bg-foreground text-background hover:opacity-90"
                >
                  commit
                </button>
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditing(false);
                  }}
                  className="text-[11px] font-mono uppercase tracking-widest px-2 py-1 border border-border hover:bg-surface-hover"
                >
                  cancel
                </button>
                <span className="font-mono text-[10px] text-muted-foreground">
                  edit mutates this stored message in place
                </span>
              </div>
            </div>
          ) : (
            <>
              <ReasoningLane reasoning={node.message.metadata?.reasoning} />

              {role === "tool" ? (
                <pre className="font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-[hsl(var(--tool-call))]/90 bg-[hsl(var(--tool-call))]/5 border border-[hsl(var(--tool-call))]/20 p-2 overflow-x-auto">
                  {textPart?.text}
                </pre>
              ) : isSystem ? (
                <div className="font-mono text-xs leading-relaxed text-muted-foreground border-l-2 border-muted-foreground/40 pl-3 py-1 italic">
                  {textPart?.text}
                </div>
              ) : (
                <MessageMarkdown text={visibleText} isStreaming={isStreaming} />
              )}

              {isLongUserMessage && (
                <button
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    setUserMessageExpanded((current) => !current);
                  }}
                  className="mt-2 inline-flex items-center gap-1 font-mono text-[10px] uppercase tracking-widest text-muted-foreground hover:text-foreground transition-colors"
                >
                  {userMessageExpanded ? "show less" : "show more"}
                  <ChevronDown className={`size-3 transition-transform duration-300 ${userMessageExpanded ? "rotate-180" : ""}`} strokeWidth={1.75} />
                </button>
              )}

              {imageParts.length > 0 && (
                <div className="mt-3 flex gap-2 flex-wrap">
                  {imageParts.map((img, i) => (
                    <MessageImagePreview
                      key={i}
                      image={img}
                      testId={`msg-image-${node.id}-${i}`}
                    />
                  ))}
                </div>
              )}

              <MetadataLanes metadata={node.message.metadata} />
            </>
          )}
        </div>

        {!editing && (
          <div className="absolute left-[6.25rem] bottom-3.5 z-10 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity flex items-center gap-0.5">
            <button
              data-testid={`msg-action-set-path-${node.id}`}
              title="set path head"
              onClick={(e) => {
                e.stopPropagation();
                setPathHead(node.id);
                toast.message("path set", {
                  description: "the next query uses this path; a different path creates a new session",
                });
              }}
              className="p-1 border border-transparent hover:border-border hover:bg-surface-hover"
            >
              <Target className="size-3.5" strokeWidth={1.75} />
            </button>
            <button
              data-testid={`msg-action-fork-${node.id}`}
              title="fork from this message"
              onClick={(e) => {
                e.stopPropagation();
                forkFromMessage(activeConv.id, node.id);
                toast.message("forked", { description: "new conversation created" });
              }}
              className="p-1 border border-transparent hover:border-border hover:bg-surface-hover"
            >
              <GitBranch className="size-3.5" strokeWidth={1.75} />
            </button>
            <button
              data-testid={`msg-action-truncate-${node.id}`}
              title="delete descendants after this message"
              onClick={(e) => {
                e.stopPropagation();
                truncateAfter(activeConv.id, node.id);
                toast.message("truncated", { description: "descendants deleted" });
              }}
              className="p-1 border border-transparent hover:border-border hover:bg-surface-hover"
            >
              <Scissors className="size-3.5" strokeWidth={1.75} />
            </button>
            <button
              data-testid={`msg-action-edit-${node.id}`}
              title="edit message"
              onClick={(e) => {
                e.stopPropagation();
                setEditing(true);
              }}
              className="p-1 border border-transparent hover:border-border hover:bg-surface-hover"
            >
              <Pencil className="size-3.5" strokeWidth={1.75} />
            </button>
            <button
              data-testid={`msg-action-copy-${node.id}`}
              title="copy message"
              onClick={(e) => {
                e.stopPropagation();
                void copyMessage();
              }}
              className="p-1 border border-transparent hover:border-border hover:bg-surface-hover"
            >
              <Copy className="size-3.5" strokeWidth={1.75} />
            </button>
            {!isSystem && (
              <button
                data-testid={`msg-action-remove-${node.id}`}
                title="remove message"
                onClick={(e) => {
                  e.stopPropagation();
                  if (!window.confirm("Are you sure you want to remove this message?")) return;
                  void removeMessage(activeConv.id, node.id).catch(() => {});
                  toast.message("message removed");
                }}
                className="p-1 border border-transparent hover:border-border hover:bg-surface-hover text-[hsl(var(--destructive))]"
              >
                <Trash2 className="size-3.5" strokeWidth={1.75} />
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

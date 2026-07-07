import { useEffect, useState } from "react";
import { useWindie } from "@/context/WindieContext";
import { fetchImageAsset } from "@/lib/windieApi";
import { ROLE_TOKENS } from "@/lib/mockData";
import {
  GitBranch,
  Scissors,
  Trash2,
  Pencil,
  MoreHorizontal,
  Wrench,
  Check,
  X,
  Image as ImageIcon,
} from "lucide-react";
import { toast } from "sonner";

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
  if (metadata.reasoning) {
    lanes.push(
      <div
        key="rs"
        className="border-l-2 border-[hsl(var(--reasoning))] pl-2 py-1 bg-[hsl(var(--reasoning))]/5"
      >
        <div className="font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--reasoning))]">
          reasoning
        </div>
        <div className="mt-0.5 text-xs text-muted-foreground italic leading-relaxed">
          {metadata.reasoning}
        </div>
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

export default function MessageRow({ node, index, isLast }) {
  const {
    selectedNodeId,
    setSelectedNodeId,
    activeConv,
    forkFromMessage,
    truncateAfter,
    removeMessage,
    editMessage,
    setActivePathToLeaf,
  } = useWindie();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(() => node.message.parts.find((p) => p.type === "text")?.text || "");

  const isSelected = selectedNodeId === node.id;
  const role = node.message.role;
  const isSystem = role === "system";
  const isStreaming = node.message.streaming;
  const textPart = node.message.parts.find((p) => p.type === "text");
  const imageParts = node.message.parts.filter((p) => p.type === "image");
  const siblings = node.parentId
    ? activeConv.nodes[node.parentId]?.childrenIds || []
    : [node.id];
  const hasSiblings = siblings.length > 1;

  const commitEdit = () => {
    editMessage(activeConv.id, node.id, draft);
    setEditing(false);
    toast.message("message edited", { description: "created sibling on new path" });
  };

  return (
    <div
      data-testid={`msg-row-${node.id}`}
      onClick={() => setSelectedNodeId(node.id)}
      className={`group relative border-b border-border py-3.5 px-6 transition-colors cursor-pointer ${
        isSelected ? "bg-surface" : "hover:bg-surface/40"
      }`}
    >
      {isSelected && (
        <div className="absolute left-0 top-0 bottom-0 w-[3px] bg-[hsl(var(--accent))]" />
      )}
      <div className="flex items-start gap-3">
        <div className="w-16 shrink-0 pt-0.5">
          <RoleBadge role={role} />
          <div className="mt-1 font-mono text-[10px] text-muted-foreground/70">
            #{String(index).padStart(2, "0")}
          </div>
          {hasSiblings && (
            <div className="mt-1 font-mono text-[10px] text-[hsl(var(--accent))]">
              {siblings.indexOf(node.id) + 1}/{siblings.length}
            </div>
          )}
        </div>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1.5 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            {node.message.model && <span className="text-foreground/70">{node.message.model}</span>}
            {node.message.tokens && <span>· {node.message.tokens}tok</span>}
            {node.message.metadata?.toolCallId && (
              <span className="text-[hsl(var(--tool-call))]">
                · call {node.message.metadata.toolCallId}
              </span>
            )}
            {node.message.timestamp && (
              <span className="ml-auto text-muted-foreground/60">
                {new Date(node.message.timestamp).toLocaleTimeString([], {
                  hour: "2-digit",
                  minute: "2-digit",
                  second: "2-digit",
                })}
              </span>
            )}
          </div>

          {editing ? (
            <div className="space-y-2">
              <textarea
                data-testid={`msg-edit-textarea-${node.id}`}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onClick={(e) => e.stopPropagation()}
                autoFocus
                rows={Math.min(12, Math.max(3, draft.split("\n").length + 1))}
                className="w-full bg-transparent border border-foreground/60 p-2 font-mono text-xs leading-relaxed outline-none resize-none"
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
              {role === "tool" ? (
                <pre className="font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-[hsl(var(--tool-call))]/90 bg-[hsl(var(--tool-call))]/5 border border-[hsl(var(--tool-call))]/20 p-2 overflow-x-auto">
                  {textPart?.text}
                </pre>
              ) : isSystem ? (
                <div className="font-mono text-xs leading-relaxed text-muted-foreground border-l-2 border-muted-foreground/40 pl-3 py-1 italic">
                  {textPart?.text}
                </div>
              ) : (
                <div className="text-sm leading-relaxed whitespace-pre-wrap font-sans">
                  {textPart?.text}
                  {isStreaming && <span className="windie-caret" />}
                </div>
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
          <div className="opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity flex items-center gap-0.5">
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
            {!isSystem && (
              <button
                data-testid={`msg-action-remove-${node.id}`}
                title="remove message"
                onClick={(e) => {
                  e.stopPropagation();
                  removeMessage(activeConv.id, node.id);
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

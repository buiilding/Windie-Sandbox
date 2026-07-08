import { useState, useRef, useEffect, useMemo } from "react";
import { useWindie } from "@/context/WindieContext";
import {
  Send,
  Paperclip,
  X,
  ChevronDown,
  Square,
  Play,
  RefreshCw,
  Image as ImageIcon,
} from "lucide-react";
import { toast } from "sonner";

function attachmentId() {
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function formatBytes(bytes = 0) {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)}mb`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)}kb`;
  return `${bytes}b`;
}

export default function Composer() {
  const {
    activeConv,
    sendMessage,
    continueConversation,
    streaming,
    setConversationModel,
    models,
    modelsLoading,
    modelsError,
    refreshModels,
    loadModelParameters,
    activeModelParameters,
    activeReasoning,
    setConversationReasoningEffort,
  } = useWindie();
  const [text, setText] = useState("");
  const [attachments, setAttachments] = useState([]);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [reasoningMenuOpen, setReasoningMenuOpen] = useState(false);
  const [modelSearch, setModelSearch] = useState("");
  const taRef = useRef(null);
  const attachmentsRef = useRef([]);

  useEffect(() => {
    if (taRef.current) {
      taRef.current.style.height = "0px";
      taRef.current.style.height = Math.min(200, taRef.current.scrollHeight) + "px";
    }
  }, [text]);

  useEffect(() => {
    attachmentsRef.current = attachments;
  }, [attachments]);

  useEffect(
    () => () => {
      attachmentsRef.current.forEach((attachment) => {
        if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
      });
    },
    []
  );

  const currentModel = activeConv?.model;
  const reasoningOptions = useMemo(
    () => activeModelParameters?.data?.reasoning?.options || [],
    [activeModelParameters]
  );
  const selectedReasoningEffort = activeReasoning?.effort || "";
  const selectedReasoningLabel = useMemo(
    () =>
      reasoningOptions.find((option) => option.value === selectedReasoningEffort)?.label ||
      "model",
    [reasoningOptions, selectedReasoningEffort]
  );
  const filteredModels = models.filter((model) =>
    model.id.toLowerCase().includes(modelSearch.trim().toLowerCase())
  );
  const hasSendContent = Boolean(text.trim() || attachments.length);

  useEffect(() => {
    if (!activeConv || !selectedReasoningEffort) return;
    if (reasoningOptions.some((option) => option.value === selectedReasoningEffort)) return;
    setConversationReasoningEffort(activeConv.id, null);
  }, [
    activeConv,
    reasoningOptions,
    selectedReasoningEffort,
    setConversationReasoningEffort,
  ]);

  const clearAttachments = () => {
    setAttachments((current) => {
      current.forEach((attachment) => {
        if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
      });
      return [];
    });
  };

  const removeAttachment = (attachmentIdToRemove) => {
    setAttachments((current) =>
      current.filter((attachment) => {
        if (attachment.id !== attachmentIdToRemove) return true;
        if (attachment.previewUrl) URL.revokeObjectURL(attachment.previewUrl);
        return false;
      })
    );
  };

  const handlePaste = (event) => {
    const pastedImages = Array.from(event.clipboardData?.items || [])
      .filter((item) => item.kind === "file" && item.type.startsWith("image/"))
      .map((item) => item.getAsFile())
      .filter(Boolean);

    if (pastedImages.length === 0) return;

    event.preventDefault();
    setAttachments((current) => [
      ...current,
      ...pastedImages.map((file) => ({
        id: attachmentId(),
        source: "clipboard",
        file,
        name: file.name || "clipboard image",
        mimeType: file.type || "image/png",
        size: file.size || 0,
        previewUrl: URL.createObjectURL(file),
      })),
    ]);
  };

  const submit = async () => {
    if (!hasSendContent || streaming) return;
    const sentText = text;
    const sentAttachments = attachments;
    setText("");
    clearAttachments();
    await sendMessage(activeConv.id, sentText, { attachments: sentAttachments });
  };

  const continueQuery = () => {
    if (!activeConv || streaming) return;
    continueConversation(activeConv.id);
  };

  const refreshModelList = () => {
    refreshModels().catch((error) => toast.error(error.message));
  };

  return (
    <div className="border-t border-border bg-background" data-testid="composer">
      <div className="px-6 py-3 flex items-start gap-3">
        <div className="flex-1 min-w-0">
          <textarea
            ref={taRef}
            data-testid="composer-textarea"
            value={text}
            onChange={(e) => setText(e.target.value)}
            onPaste={handlePaste}
            onKeyDown={(e) => {
              if (
                e.key === "Enter" &&
                !e.shiftKey &&
                !e.nativeEvent.isComposing
              ) {
                e.preventDefault();
                submit();
              }
            }}
            placeholder="query the runtime. enter to send, shift + enter for newline."
            rows={2}
            className="w-full bg-transparent outline-none resize-none font-mono text-[13px] leading-relaxed placeholder:text-muted-foreground/60"
          />

          {attachments.length > 0 && (
            <div className="mt-2 flex gap-2 overflow-x-auto pb-1">
              {attachments.map((attachment) => (
                <div
                  key={attachment.id}
                  className="w-40 shrink-0 border border-border bg-surface/40 p-1"
                >
                  <div className="h-20 border border-border/60 bg-background flex items-center justify-center overflow-hidden">
                    {attachment.previewUrl ? (
                      <img
                        src={attachment.previewUrl}
                        alt={attachment.name || "attachment"}
                        className="h-full w-full object-cover"
                      />
                    ) : (
                      <ImageIcon className="size-5 text-muted-foreground" />
                    )}
                  </div>
                  <div className="mt-1 flex items-center gap-1.5 min-w-0">
                    <ImageIcon className="size-3 shrink-0 text-muted-foreground" />
                    <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-muted-foreground">
                      {attachment.source === "path"
                        ? attachment.path
                        : `${attachment.mimeType} · ${formatBytes(attachment.size)}`}
                    </span>
                    <button
                      type="button"
                      onClick={() => removeAttachment(attachment.id)}
                      className="shrink-0 text-muted-foreground hover:text-foreground"
                      aria-label="remove attachment"
                    >
                      <X className="size-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}

          <div className="mt-2 flex items-center gap-2">
            <button
              data-testid="composer-attach-image"
              onClick={() => {
                const nextPath = window.prompt("Local image path");
                if (nextPath?.trim()) {
                  setAttachments((current) => [
                    ...current,
                    {
                      id: attachmentId(),
                      source: "path",
                      path: nextPath.trim(),
                      name: nextPath.trim().split("/").pop() || "image path",
                    },
                  ]);
                }
              }}
              className="h-7 px-2 flex items-center gap-1.5 border border-border text-muted-foreground hover:bg-surface-hover transition-colors font-mono text-[11px] uppercase tracking-widest"
            >
              <Paperclip className="size-3.5" strokeWidth={1.75} />
              attach image
            </button>

            <div className="relative">
              <button
                data-testid="composer-model"
                onClick={() => setModelMenuOpen(!modelMenuOpen)}
                className="h-7 max-w-[440px] px-2 flex items-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[11px] uppercase tracking-widest"
              >
                <span className="text-muted-foreground">model</span>
                <span className="min-w-0 truncate text-foreground normal-case">{currentModel}</span>
                <ChevronDown className="size-3" />
              </button>
              {modelMenuOpen && (
                <>
                  <div
                    className="fixed inset-0 z-10"
                    onClick={() => setModelMenuOpen(false)}
                  />
                  <div className="absolute bottom-full mb-1 left-0 z-20 w-[420px] max-w-[calc(100vw-3rem)] bg-popover border border-border shadow-md">
                    <div className="px-2.5 py-1.5 border-b border-border font-mono text-[10px] uppercase tracking-widest text-muted-foreground flex items-center justify-between gap-2">
                      <span>model</span>
                      <button
                        type="button"
                        onClick={(event) => {
                          event.stopPropagation();
                          refreshModelList();
                        }}
                        className="size-6 inline-flex items-center justify-center border border-transparent hover:border-border hover:bg-surface-hover text-muted-foreground"
                        title="refresh models"
                        aria-label="refresh models"
                      >
                        <RefreshCw className={`size-3.5 ${modelsLoading ? "animate-spin" : ""}`} strokeWidth={1.75} />
                      </button>
                    </div>
                    <div className="p-2 border-b border-border">
                      <input
                        data-testid="composer-model-filter"
                        value={modelSearch}
                        onChange={(event) => setModelSearch(event.target.value)}
                        placeholder="filter models"
                        className="h-8 w-full bg-background border border-border px-2 font-mono text-xs outline-none placeholder:text-muted-foreground/60 focus:border-[hsl(var(--accent))]"
                      />
                    </div>
                    <div className="max-h-[260px] overflow-y-auto">
                      {modelsLoading && (
                        <div className="px-2.5 py-2 text-xs font-mono text-muted-foreground">
                          loading models
                        </div>
                      )}
                      {!modelsLoading && modelsError && (
                        <div className="px-2.5 py-2 text-xs font-mono text-muted-foreground">
                          models unavailable
                        </div>
                      )}
                      {!modelsLoading && !modelsError && filteredModels.length === 0 && (
                        <div className="px-2.5 py-2 text-xs font-mono text-muted-foreground">
                          no models
                        </div>
                      )}
                      {!modelsLoading && !modelsError && filteredModels.map((m) => (
                        <button
                          key={m.id}
                          data-testid={`composer-model-option-${m.id}`}
                          onMouseEnter={() => loadModelParameters(m.id)}
                          onClick={() => {
                            if (activeConv) {
                              setConversationModel(activeConv.id, m.id).catch((error) =>
                                toast.error(error.message)
                              );
                            }
                            setModelMenuOpen(false);
                            setModelSearch("");
                          }}
                          className={`w-full text-left px-2.5 py-1.5 text-xs font-mono hover:bg-surface-hover flex items-center justify-between gap-3 ${
                            currentModel === m.id ? "bg-surface" : ""
                          }`}
                        >
                          <span className="min-w-0 truncate">{m.label}</span>
                          <span className="shrink-0 text-muted-foreground uppercase text-[10px]">
                            {m.id.split("/")[0]}
                          </span>
                        </button>
                      ))}
                    </div>
                  </div>
                </>
              )}
            </div>

            {reasoningOptions.length > 0 && (
              <div className="relative">
                <button
                  data-testid="composer-reasoning"
                  type="button"
                  onClick={() => setReasoningMenuOpen(!reasoningMenuOpen)}
                  disabled={!activeConv || streaming}
                  className="h-7 max-w-[260px] px-2 flex items-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[11px] uppercase tracking-widest disabled:text-muted-foreground disabled:hover:bg-transparent disabled:cursor-not-allowed"
                >
                  <span className="text-muted-foreground">reasoning</span>
                  <span className="min-w-0 truncate text-foreground normal-case">
                    {selectedReasoningLabel}
                  </span>
                  <ChevronDown className="size-3" />
                </button>
                {reasoningMenuOpen && (
                  <>
                    <div
                      className="fixed inset-0 z-10"
                      onClick={() => setReasoningMenuOpen(false)}
                    />
                    <div className="absolute bottom-full mb-1 left-0 z-20 w-[240px] max-w-[calc(100vw-3rem)] bg-popover border border-border shadow-md">
                      <div className="px-2.5 py-1.5 border-b border-border font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
                        reasoning
                      </div>
                      <div className="max-h-[240px] overflow-y-auto py-1">
                        {reasoningOptions.map((option) => (
                          <button
                            key={option.value}
                            type="button"
                            onClick={() => {
                              setConversationReasoningEffort(activeConv?.id, option.value);
                              setReasoningMenuOpen(false);
                            }}
                            className={`w-full text-left px-2.5 py-1.5 text-xs font-mono hover:bg-surface-hover flex items-center justify-between gap-3 ${
                              selectedReasoningEffort === option.value ? "bg-surface" : ""
                            }`}
                          >
                            <span className="min-w-0 truncate">{option.label}</span>
                          </button>
                        ))}
                      </div>
                    </div>
                  </>
                )}
              </div>
            )}

            <div className="flex-1" />
            <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
              {text.length}c · {text.split(/\s+/).filter(Boolean).length}w
            </span>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            data-testid="composer-continue"
            onClick={continueQuery}
            disabled={streaming || !activeConv}
            className={`h-10 px-4 flex items-center gap-2 border font-mono text-xs uppercase tracking-widest transition-colors ${
              streaming || !activeConv
                ? "border-border text-muted-foreground cursor-not-allowed"
                : "border-border text-foreground hover:bg-surface-hover"
            }`}
          >
            {streaming ? (
              <>
                <Square className="size-3 fill-current" />
                busy
              </>
            ) : (
              <>
                <Play className="size-3.5" strokeWidth={1.75} />
                continue
              </>
            )}
          </button>

          <button
            data-testid="composer-send"
            onClick={submit}
            disabled={streaming || !hasSendContent}
            className={`h-10 px-4 flex items-center gap-2 border font-mono text-xs uppercase tracking-widest transition-colors ${
              streaming
                ? "border-[hsl(var(--accent))] text-[hsl(var(--accent))] cursor-not-allowed"
                : hasSendContent
                  ? "border-foreground bg-foreground text-background hover:opacity-90"
                  : "border-border text-muted-foreground cursor-not-allowed"
            }`}
          >
            {streaming ? (
              <>
                <Square className="size-3 fill-current" />
                streaming
              </>
            ) : (
              <>
                <Send className="size-3.5" strokeWidth={1.75} />
                query
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
}

import { useState, useRef, useEffect } from "react";
import { useWindie } from "@/context/WindieContext";
import {
  Send,
  Paperclip,
  X,
  ChevronDown,
  Square,
  Play,
  RefreshCw,
} from "lucide-react";
import { toast } from "sonner";

export default function Composer() {
  const {
    activeConv,
    sendMessage,
    continueConversation,
    streaming,
    modelOverride,
    setModelOverride,
    models,
    modelsLoading,
    modelsError,
    refreshModels,
  } = useWindie();
  const [text, setText] = useState("");
  const [imagePath, setImagePath] = useState("");
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [modelSearch, setModelSearch] = useState("");
  const taRef = useRef(null);

  useEffect(() => {
    if (taRef.current) {
      taRef.current.style.height = "0px";
      taRef.current.style.height = Math.min(200, taRef.current.scrollHeight) + "px";
    }
  }, [text]);

  const currentModel = modelOverride || activeConv?.model;
  const filteredModels = models.filter((model) =>
    model.id.toLowerCase().includes(modelSearch.trim().toLowerCase())
  );

  const submit = () => {
    if (!text.trim() || streaming) return;
    sendMessage(activeConv.id, text, { modelOverride, imagePath });
    setText("");
    setImagePath("");
  };

  const continueQuery = () => {
    if (!activeConv || streaming) return;
    continueConversation(activeConv.id, { modelOverride });
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

          <div className="mt-2 flex items-center gap-2">
            <button
              data-testid="composer-attach-image"
              onClick={() => {
                if (imagePath) {
                  setImagePath("");
                  return;
                }
                const nextPath = window.prompt("Local image path");
                if (nextPath?.trim()) setImagePath(nextPath.trim());
              }}
              className={`h-7 px-2 flex items-center gap-1.5 border transition-colors font-mono text-[11px] uppercase tracking-widest ${
                imagePath
                  ? "border-[hsl(var(--accent))] text-[hsl(var(--accent))] bg-[hsl(var(--accent))]/10"
                  : "border-border text-muted-foreground hover:bg-surface-hover"
              }`}
            >
              <Paperclip className="size-3.5" strokeWidth={1.75} />
              {imagePath ? "image path set" : "attach image"}
              {imagePath && (
                <X
                  className="size-3 ml-1"
                  onClick={(e) => {
                    e.stopPropagation();
                    setImagePath("");
                  }}
                />
              )}
            </button>

            <div className="relative">
              <button
                data-testid="composer-model-override"
                onClick={() => setModelMenuOpen(!modelMenuOpen)}
                className="h-7 max-w-[440px] px-2 flex items-center gap-1.5 border border-border hover:bg-surface-hover font-mono text-[11px] uppercase tracking-widest"
              >
                <span className="text-muted-foreground">model</span>
                <span className="min-w-0 truncate text-foreground normal-case">{currentModel}</span>
                {modelOverride && (
                  <span className="text-[hsl(var(--accent))] normal-case">· override</span>
                )}
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
                      <span>model override</span>
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
                    <button
                      data-testid="composer-model-option-inherit"
                      onClick={() => {
                        setModelOverride(null);
                        setModelMenuOpen(false);
                      }}
                      className="w-full text-left px-2.5 py-1.5 text-xs font-mono hover:bg-surface-hover flex items-center justify-between"
                    >
                      <span>default</span>
                      <span className="ml-3 min-w-0 truncate text-muted-foreground">{activeConv?.model}</span>
                    </button>
                    <div className="border-t border-border" />
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
                          onClick={() => {
                            setModelOverride(m.id);
                            setModelMenuOpen(false);
                            setModelSearch("");
                          }}
                          className={`w-full text-left px-2.5 py-1.5 text-xs font-mono hover:bg-surface-hover flex items-center justify-between gap-3 ${
                            modelOverride === m.id ? "bg-surface" : ""
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
            disabled={streaming || !text.trim()}
            className={`h-10 px-4 flex items-center gap-2 border font-mono text-xs uppercase tracking-widest transition-colors ${
              streaming
                ? "border-[hsl(var(--accent))] text-[hsl(var(--accent))] cursor-not-allowed"
                : text.trim()
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

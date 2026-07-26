import { useCallback, useEffect, useState } from "react";
import { Check, CheckCircle2, Loader2, Plus } from "lucide-react";
import { toast } from "sonner";
import {
  createLlmProviderKey,
  ensureLlmProvider,
  listLlmProviders,
} from "@/lib/windieApi";

function providerState(provider) {
  if (provider.authentication === "none") {
    return { kind: "auto", label: "no key needed" };
  }
  if (provider.configuration !== "simple" || provider.authentication !== "api_key") {
    return { kind: "unsupported", label: "structured setup required" };
  }
  return { kind: "key", label: null };
}

function ProviderRow({ provider, selected, onToggle }) {
  const state = providerState(provider);
  const disabled = state.kind === "unsupported";

  return (
    <button
      type="button"
      data-testid={`llm-provider-select-${provider.name}`}
      disabled={disabled}
      onClick={() => onToggle(provider.name)}
      className={`flex w-full items-center justify-between gap-3 border px-3 py-2 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        selected
          ? "border-foreground bg-surface/60"
          : "border-border hover:bg-surface-hover"
      }`}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate font-mono text-[11px] text-foreground">
          {provider.display_name}
        </span>
        <span className="block font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
          {state.label ||
            (provider.configured
              ? `configured · ${provider.key_count} key${provider.key_count === 1 ? "" : "s"}`
              : "not configured")}
        </span>
      </span>
      <span
        className={`grid size-4 shrink-0 place-items-center border ${
          selected ? "border-foreground bg-foreground text-background" : "border-border"
        }`}
      >
        {selected && <Check className="size-3" strokeWidth={2.5} />}
      </span>
    </button>
  );
}

function ProviderKeyForm({ provider, onSaved }) {
  const [keyName, setKeyName] = useState("");
  const [keyValue, setKeyValue] = useState("");
  const [pending, setPending] = useState(false);
  const [saved, setSaved] = useState(false);

  const defaultName = `windie-${provider.name}-${provider.key_count + 1}`;

  const save = async () => {
    if (pending || !keyValue.trim()) return;
    setPending(true);
    try {
      if (!provider.configured) {
        await ensureLlmProvider(provider.name);
      }
      await createLlmProviderKey(provider.name, {
        name: keyName.trim() || defaultName,
        value: keyValue.trim(),
      });
      setSaved(true);
      setKeyValue("");
      toast.message("provider key saved", { description: provider.display_name });
      onSaved();
    } catch (error) {
      toast.error("failed to save provider key", {
        description: error?.message || String(error),
      });
    } finally {
      setPending(false);
    }
  };

  if (provider.authentication === "none") {
    return (
      <div className="flex items-center gap-2 border border-t-0 border-border bg-surface/30 px-3 py-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        <CheckCircle2 className="size-3.5 text-[hsl(var(--tool-call))]" />
        available without a key
      </div>
    );
  }

  return (
    <div className="space-y-2 border border-t-0 border-border bg-surface/30 px-3 py-3">
      <div className="grid gap-2 sm:grid-cols-[1fr_2fr]">
        <input
          type="text"
          data-testid={`llm-key-name-${provider.name}`}
          value={keyName}
          onChange={(event) => setKeyName(event.target.value)}
          placeholder={defaultName}
          autoComplete="off"
          data-1p-ignore
          data-lpignore="true"
          className="h-8 border border-border bg-background px-2 font-mono text-[11px] outline-none focus:border-foreground"
        />
        <input
          type="password"
          data-testid={`llm-key-value-${provider.name}`}
          value={keyValue}
          onChange={(event) => {
            setKeyValue(event.target.value);
            setSaved(false);
          }}
          placeholder={`${provider.display_name} API key`}
          autoComplete="new-password"
          data-1p-ignore
          data-lpignore="true"
          className="h-8 border border-border bg-background px-2 font-mono text-[11px] outline-none focus:border-foreground"
        />
      </div>
      <div className="flex items-center justify-between">
        <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
          stored by bifrost, not windie
        </span>
        <button
          type="button"
          data-testid={`llm-key-save-${provider.name}`}
          disabled={pending || !keyValue.trim()}
          onClick={save}
          className="inline-flex h-7 items-center gap-1.5 border border-foreground bg-foreground px-3 font-mono text-[10px] uppercase tracking-widest text-background transition-opacity disabled:cursor-not-allowed disabled:opacity-50"
        >
          {pending ? (
            <Loader2 className="size-3 animate-spin" />
          ) : saved ? (
            <CheckCircle2 className="size-3" />
          ) : (
            <Plus className="size-3" />
          )}
          {pending ? "saving" : saved ? "saved" : "save key"}
        </button>
      </div>
    </div>
  );
}

export default function LlmProvidersPanel() {
  const [providers, setProviders] = useState([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState([]);

  const refresh = useCallback(async () => {
    try {
      const catalog = await listLlmProviders();
      setProviders(catalog);
    } catch (error) {
      toast.error("failed to load llm providers", {
        description: error?.message || String(error),
      });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const toggle = (name) =>
    setSelected((current) =>
      current.includes(name)
        ? current.filter((entry) => entry !== name)
        : [...current, name]
    );

  if (loading) {
    return (
      <div className="flex items-center justify-center gap-2 py-12 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin" />
        loading providers
      </div>
    );
  }

  return (
    <div className="p-3" data-testid="llm-providers-panel">
      <div className="mb-2 flex items-center justify-between px-1">
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          llm providers · {providers.length}
        </span>
        <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
          select to configure
        </span>
      </div>
      <div className="space-y-2">
        {providers.map((provider) => {
          const isSelected = selected.includes(provider.name);
          return (
            <div key={provider.name}>
              <ProviderRow provider={provider} selected={isSelected} onToggle={toggle} />
              {isSelected && providerState(provider).kind !== "unsupported" && (
                <ProviderKeyForm provider={provider} onSaved={refresh} />
              )}
            </div>
          );
        })}
        {providers.length === 0 && (
          <div className="border border-border px-3 py-6 text-center font-mono text-[11px] text-muted-foreground">
            no providers in the bifrost catalog
          </div>
        )}
      </div>
    </div>
  );
}

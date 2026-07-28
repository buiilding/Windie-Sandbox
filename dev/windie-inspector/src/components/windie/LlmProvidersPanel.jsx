import { useCallback, useEffect, useState } from "react";
import {
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Loader2,
  Plus,
  Save,
  Trash2,
} from "lucide-react";
import { toast } from "sonner";
import {
  createLlmProviderKey,
  deleteLlmProviderKey,
  ensureLlmProvider,
  listLlmProviderKeys,
  listLlmProviders,
} from "@/lib/windieApi";

function providerState(provider, keys = []) {
  if (provider.authentication === "none") {
    return { kind: "ready", label: "ready · no key needed" };
  }
  if (provider.configuration !== "simple" || provider.authentication !== "api_key") {
    return { kind: "unsupported", label: "structured setup required" };
  }
  if (provider.key_count > 0) {
    const invalidKeyCount = keys.filter((key) => key.status === "list_models_failed").length;
    if (invalidKeyCount === keys.length && invalidKeyCount > 0) {
      return { kind: "invalid", label: "invalid key · check key" };
    }
    if (invalidKeyCount > 0) {
      return {
        kind: "ready",
        label: `ready · ${provider.key_count} key${provider.key_count === 1 ? "" : "s"} · ${invalidKeyCount} invalid`,
      };
    }
    return {
      kind: "ready",
      label: `ready · ${provider.key_count} key${provider.key_count === 1 ? "" : "s"}`,
    };
  }
  return { kind: "needs-key", label: "needs API key" };
}

function ProviderRow({ provider, keys, expanded, onToggle }) {
  const state = providerState(provider, keys);
  const disabled = state.kind === "unsupported";

  return (
    <button
      type="button"
      data-testid={`llm-provider-select-${provider.name}`}
      disabled={disabled}
      onClick={() => onToggle(provider.name)}
      className={`flex w-full items-center justify-between gap-3 border px-3 py-2 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        expanded
          ? "border-foreground bg-surface/60"
          : "border-border hover:bg-surface-hover"
      }`}
    >
      <span className="flex min-w-0 items-center gap-2">
        {expanded ? (
          <ChevronDown className="size-3 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRight className="size-3 shrink-0 text-muted-foreground" />
        )}
        <span className="min-w-0 flex-1">
          <span className="block truncate font-mono text-[11px] text-foreground">
            {provider.display_name}
          </span>
          <span
            className={`block font-mono text-[9px] uppercase tracking-widest ${
              state.kind === "ready"
                ? "text-[hsl(var(--tool-call))]"
                : state.kind === "needs-key" || state.kind === "invalid"
                  ? "text-[hsl(var(--destructive))]"
                  : "text-muted-foreground"
            }`}
          >
            {state.label}
          </span>
        </span>
      </span>
      <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
        {expanded ? "close" : "manage keys"}
      </span>
    </button>
  );
}

function ProviderKeyForm({ provider, onSaved, onModelsChanged, onCancel }) {
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
      await onSaved();
      await onModelsChanged?.();
    } catch (error) {
      toast.error("failed to save provider key", {
        description: error?.message || String(error),
      });
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="space-y-2 border border-t-0 border-border bg-surface/30 px-3 py-3">
      <div className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
        add API key
      </div>
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
      <div className="flex items-center justify-between gap-2">
        <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
          stored by bifrost, not windie
        </span>
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onCancel}
            className="h-7 border border-border px-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground hover:bg-surface-hover"
          >
            cancel
          </button>
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
              <Save className="size-3" />
            )}
            {pending ? "saving" : saved ? "saved" : "save"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ProviderManagement({ provider, keys, keysLoaded, keysError, onRefresh, onModelsChanged }) {
  const [showAddKey, setShowAddKey] = useState(provider.key_count === 0);
  const [pendingKeyId, setPendingKeyId] = useState(null);

  const refreshProviderState = async () => {
    try {
      await onModelsChanged?.();
    } catch (error) {
      toast.error("failed to refresh model catalog", {
        description: error?.message || String(error),
      });
    }
    await onRefresh();
  };

  const deleteKey = async (key) => {
    if (pendingKeyId || !window.confirm(`Delete the ${key.name} API key?`)) return;
    setPendingKeyId(key.id);
    try {
      await deleteLlmProviderKey(provider.name, key.id);
      toast.message("provider key deleted", { description: provider.display_name });
      await refreshProviderState();
    } catch (error) {
      toast.error("failed to delete provider key", {
        description: error?.message || String(error),
      });
    } finally {
      setPendingKeyId(null);
    }
  };

  if (provider.authentication === "none") {
    return (
      <div className="flex items-center gap-2 border border-t-0 border-border bg-surface/30 px-3 py-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
        <CheckCircle2 className="size-3.5 text-[hsl(var(--tool-call))]" />
        available without an API key
      </div>
    );
  }

  return (
    <div className="space-y-2 border border-t-0 border-border bg-surface/30 px-3 py-3">
      <div className="flex items-center justify-between">
        <span className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
          API keys
        </span>
        <button
          type="button"
          onClick={() => setShowAddKey((current) => !current)}
          className="inline-flex h-7 items-center border border-border px-2 font-mono text-[10px] uppercase tracking-widest hover:bg-surface-hover"
        >
          {showAddKey ? "close" : "add key"}
        </button>
      </div>

      {keysError ? (
        <div className="flex items-center justify-between gap-2 border border-border px-2 py-2 font-mono text-[10px] uppercase tracking-widest text-[hsl(var(--destructive))]">
          unable to load keys
          <button
            type="button"
            onClick={onRefresh}
            className="border border-border px-2 py-1 text-muted-foreground hover:bg-surface-hover"
          >
            retry
          </button>
        </div>
      ) : !keysLoaded ? (
        <div className="flex items-center gap-2 py-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          <Loader2 className="size-3 animate-spin" />
          loading keys
        </div>
      ) : keys.length === 0 ? (
        <div className="border border-border px-2 py-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          no API keys stored
        </div>
      ) : (
        <div className="space-y-1">
          {keys.map((key) => (
            <div
              key={key.id}
              className="flex items-center justify-between gap-2 border border-border bg-background px-2 py-2"
            >
              <div className="min-w-0">
                <div className="truncate font-mono text-[11px] text-foreground">
                  {key.name || key.id}
                </div>
                <div className="font-mono text-[9px] uppercase tracking-widest text-muted-foreground">
                  {key.status === "list_models_failed"
                    ? "invalid · check key"
                    : key.enabled === false
                      ? "disabled"
                      : "active"}
                </div>
              </div>
              <button
                type="button"
                aria-label={`delete ${key.name || "API key"}`}
                disabled={pendingKeyId === key.id}
                onClick={() => deleteKey(key)}
                className="inline-flex size-7 shrink-0 items-center justify-center border border-border text-[hsl(var(--destructive))] hover:bg-surface-hover disabled:opacity-50"
              >
                {pendingKeyId === key.id ? (
                  <Loader2 className="size-3 animate-spin" />
                ) : (
                  <Trash2 className="size-3" />
                )}
              </button>
            </div>
          ))}
        </div>
      )}

      {showAddKey && (
        <ProviderKeyForm
          provider={provider}
          onSaved={async () => {
            await onRefresh();
            setShowAddKey(false);
          }}
          onModelsChanged={refreshProviderState}
          onCancel={() => setShowAddKey(false)}
        />
      )}
    </div>
  );
}

export default function LlmProvidersPanel({ onModelsChanged }) {
  const [providers, setProviders] = useState([]);
  const [keysByProvider, setKeysByProvider] = useState({});
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState([]);

  const refresh = useCallback(async () => {
    try {
      setLoading(true);
      const catalog = await listLlmProviders();
      const keyEntries = await Promise.all(
        catalog.map(async (provider) => {
          if (provider.key_count === 0 || provider.authentication === "none") {
            return [provider.name, []];
          }
          try {
            return [provider.name, await listLlmProviderKeys(provider.name)];
          } catch {
            return [provider.name, null];
          }
        })
      );
      const nextKeysByProvider = Object.fromEntries(keyEntries);
      const sortedCatalog = [...catalog].sort((left, right) => {
        const rank = (provider) => {
          const kind = providerState(provider, nextKeysByProvider[provider.name] || []).kind;
          return kind === "ready" ? 0 : kind === "needs-key" ? 1 : 2;
        };
        const leftRank = rank(left);
        const rightRank = rank(right);
        return leftRank - rightRank || left.display_name.localeCompare(right.display_name);
      });
      setProviders(sortedCatalog);
      setKeysByProvider(nextKeysByProvider);
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
          manage credentials
        </span>
      </div>
      <div className="space-y-2">
        {providers.map((provider) => {
          const expanded = selected.includes(provider.name);
          const providerKeys = keysByProvider[provider.name];
          return (
            <div key={provider.name}>
              <ProviderRow
                provider={provider}
                keys={providerKeys || []}
                expanded={expanded}
                onToggle={toggle}
              />
              {expanded && (
                <ProviderManagement
                  provider={provider}
                  keys={providerKeys || []}
                  keysLoaded={providerKeys !== undefined && providerKeys !== null}
                  keysError={providerKeys === null}
                  onRefresh={refresh}
                  onModelsChanged={onModelsChanged}
                />
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

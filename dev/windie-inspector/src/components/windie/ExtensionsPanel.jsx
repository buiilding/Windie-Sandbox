import { useMemo, useState } from "react";
import {
  AlertTriangle,
  Box,
  Check,
  CheckCircle2,
  Download,
  Globe2,
  HardDrive,
  Loader2,
  MonitorCog,
  MousePointer2,
  Network,
  PackageOpen,
  Power,
  ShieldCheck,
  Trash2,
  Wrench,
} from "lucide-react";
import { toast } from "sonner";
import { useWindie } from "@/context/WindieContext";

const providerIcons = {
  "desktop-commander": MonitorCog,
  "cua-driver": MousePointer2,
  "blender-mcp": Box,
  brightdata: Globe2,
};

const permissionIcons = {
  computer_control: MousePointer2,
  external_process: Power,
  filesystem: HardDrive,
  network: Network,
};

const permissionLabels = {
  computer_control: "computer control",
  external_process: "external process",
  filesystem: "filesystem",
  network: "network",
};

function providerStatus(provider, toolStatus) {
  const state = provider.installation?.state;
  if (!state) {
    return { label: "not installed", tone: "muted", icon: PackageOpen };
  }
  if (state === "enabled") {
    return { label: "enabled", tone: "good", icon: CheckCircle2 };
  }
  if (state === "disabled") {
    return { label: "disabled", tone: "muted", icon: Power };
  }
  if (state === "broken") {
    return { label: "needs repair", tone: "bad", icon: AlertTriangle };
  }
  if (state === "updating") {
    return { label: "setting up", tone: "accent", icon: Loader2 };
  }
  if (toolStatus && !toolStatus.available) {
    return { label: "not responding", tone: "bad", icon: AlertTriangle };
  }
  return { label: "installed", tone: "muted", icon: PackageOpen };
}

function StatusBadge({ status }) {
  const StatusIcon = status.icon;
  const tone = {
    good: "text-[hsl(var(--tool-call))] border-[hsl(var(--tool-call))]/30 bg-[hsl(var(--tool-call))]/8",
    bad: "text-[hsl(var(--destructive))] border-[hsl(var(--destructive))]/30 bg-[hsl(var(--destructive))]/8",
    accent: "text-accent border-accent/30 bg-accent/8",
    muted: "text-muted-foreground border-border bg-surface/40",
  }[status.tone];

  return (
    <span className={`inline-flex items-center gap-1 border px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider ${tone}`}>
      <StatusIcon className={`size-3 ${status.tone === "accent" ? "animate-spin" : ""}`} strokeWidth={1.75} />
      {status.label}
    </span>
  );
}

function PermissionChip({ permission }) {
  const Icon = permissionIcons[permission] || ShieldCheck;
  return (
    <span className="inline-flex items-center gap-1 border border-border bg-surface/35 px-1.5 py-1 font-mono text-[9px] uppercase tracking-wide text-muted-foreground">
      <Icon className="size-3" strokeWidth={1.5} />
      {permissionLabels[permission] || permission.replaceAll("_", " ")}
    </span>
  );
}

function ProviderCard({ provider, toolStatus, pending, onAction }) {
  const Icon = providerIcons[provider.providerId] || ShieldCheck;
  const status = providerStatus(provider, toolStatus);
  const installed = Boolean(provider.installation);
  const state = provider.installation?.state;
  const setupAvailable = provider.providerId === "desktop-commander";
  const toolCount = toolStatus?.toolCount ?? 0;
  const requirements = [
    ...(provider.dependencies || []).map((dependency) => dependency.executable),
    ...(provider.secrets || []).map((secret) => secret.env_key),
  ];

  return (
    <article className="group flex min-h-[250px] flex-col border border-border bg-card/60 transition-colors hover:border-muted-foreground/50 hover:bg-card">
      <div className="flex items-start gap-3 border-b border-border p-4">
        <div className="grid size-12 shrink-0 place-items-center border border-border bg-surface text-foreground shadow-sm">
          <Icon className="size-6" strokeWidth={1.35} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0">
              <h3 className="truncate font-sans text-base font-medium tracking-tight text-foreground">{provider.displayName}</h3>
              <p className="mt-0.5 truncate font-mono text-[9px] uppercase tracking-widest text-muted-foreground">{provider.providerId}</p>
            </div>
            <StatusBadge status={status} />
          </div>
        </div>
      </div>

      <div className="flex flex-1 flex-col gap-4 p-4">
        <p className="min-h-[42px] text-[12px] leading-relaxed text-muted-foreground">{provider.description}</p>

        <div className="flex flex-wrap gap-1.5">
          {(provider.permissions || []).map((permission) => (
            <PermissionChip key={permission} permission={permission} />
          ))}
        </div>

        <div className="mt-auto space-y-2 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
          <div className="flex items-center justify-between gap-2">
            <span>{installed ? `${toolCount} tools available` : "local extension"}</span>
            <span>{provider.kind || "mcp"}</span>
          </div>
          {requirements.length > 0 && (
            <div className="truncate border-t border-border pt-2" title={requirements.join(", ")}>
              requires {requirements.join(" · ")}
            </div>
          )}
        </div>
      </div>

      <div className="flex min-h-12 items-center gap-2 border-t border-border bg-surface/25 px-4 py-2">
        {!installed ? (
          <button
            type="button"
            disabled={pending || !setupAvailable}
            onClick={() => onAction("setup", provider.providerId)}
            className={`inline-flex h-8 flex-1 items-center justify-center gap-2 border px-3 font-mono text-[10px] uppercase tracking-widest transition-opacity disabled:cursor-not-allowed disabled:opacity-50 ${setupAvailable ? "border-foreground bg-foreground text-background hover:opacity-85" : "border-border text-muted-foreground"}`}
          >
            {pending ? <Loader2 className="size-3 animate-spin" /> : setupAvailable ? <Download className="size-3" /> : null}
            {pending ? "setting up" : setupAvailable ? "set up" : "setup unavailable"}
          </button>
        ) : state === "updating" || pending ? (
          <div className="flex flex-1 items-center justify-center gap-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            <Loader2 className="size-3 animate-spin" />
            working
          </div>
        ) : state === "broken" ? (
          <button
            type="button"
            onClick={() => onAction("repair", provider.providerId)}
            className="inline-flex h-8 flex-1 items-center justify-center gap-2 border border-accent bg-accent px-3 font-mono text-[10px] uppercase tracking-widest text-accent-foreground hover:opacity-85"
          >
            <Wrench className="size-3" />
            repair
          </button>
        ) : state === "enabled" ? (
          <button
            type="button"
            onClick={() => onAction("disable", provider.providerId)}
            className="inline-flex h-8 flex-1 items-center justify-center gap-2 border border-border px-3 font-mono text-[10px] uppercase tracking-widest text-muted-foreground hover:bg-surface-hover hover:text-foreground"
          >
            <Power className="size-3" />
            disable
          </button>
        ) : (
          <button
            type="button"
            onClick={() => onAction("enable", provider.providerId)}
            className="inline-flex h-8 flex-1 items-center justify-center gap-2 border border-foreground bg-foreground px-3 font-mono text-[10px] uppercase tracking-widest text-background hover:opacity-85"
          >
            <Check className="size-3" />
            enable
          </button>
        )}

        {installed && state !== "updating" && !pending && (
          <>
            <button
              type="button"
              title="repair extension"
              onClick={() => onAction("repair", provider.providerId)}
              className="grid size-8 place-items-center border border-border text-muted-foreground hover:bg-surface-hover hover:text-foreground"
            >
              <Wrench className="size-3.5" />
            </button>
            <button
              type="button"
              title="remove extension"
              onClick={() => onAction("uninstall", provider.providerId)}
              className="grid size-8 place-items-center border border-border text-muted-foreground hover:border-[hsl(var(--destructive))]/50 hover:bg-[hsl(var(--destructive))]/8 hover:text-[hsl(var(--destructive))]"
            >
              <Trash2 className="size-3.5" />
            </button>
          </>
        )}
      </div>
    </article>
  );
}

export default function ExtensionsPanel() {
  const {
    providerInstallations,
    providerInstallationsLoading,
    toolProviderStatuses,
    setupProvider,
    enableProvider,
    disableProvider,
    repairProvider,
    uninstallProvider,
  } = useWindie();
  const [pendingProviderId, setPendingProviderId] = useState(null);

  const toolStatusesById = useMemo(
    () => new Map((toolProviderStatuses || []).map((provider) => [provider.providerId, provider])),
    [toolProviderStatuses]
  );

  const runAction = async (action, providerId) => {
    if (action === "uninstall" && !window.confirm("Remove this extension from Windie?")) return;
    setPendingProviderId(providerId);
    try {
      const actions = { setup: setupProvider, enable: enableProvider, disable: disableProvider, repair: repairProvider, uninstall: uninstallProvider };
      await actions[action](providerId);
      const labels = {
        setup: "set up",
        enable: "enabled",
        disable: "disabled",
        repair: "repaired",
        uninstall: "removed",
      };
      toast.message(`extension ${labels[action]}`);
    } finally {
      setPendingProviderId(null);
    }
  };

  return (
    <div className="flex min-h-full flex-col">
      <div className="border-b border-border bg-surface/25 px-5 py-5">
        <div className="flex items-start gap-3">
          <div className="grid size-10 shrink-0 place-items-center border border-accent/50 bg-accent/10 text-accent">
            <PackageOpen className="size-5" strokeWidth={1.5} />
          </div>
          <div className="min-w-0">
            <div className="font-sans text-lg font-medium tracking-tight">Extension library</div>
            <p className="mt-1 max-w-2xl text-[12px] leading-relaxed text-muted-foreground">
              Install local capabilities once. Only enabled, healthy extensions can be attached to a conversation.
            </p>
          </div>
          <div className="ml-auto hidden shrink-0 text-right sm:block">
            <div className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">local catalog</div>
            <div className="mt-1 font-mono text-sm text-foreground">{providerInstallations.length} extensions</div>
          </div>
        </div>
      </div>

      <div className="flex-1 p-5">
        {providerInstallationsLoading && providerInstallations.length === 0 ? (
          <div className="flex min-h-48 items-center justify-center gap-2 font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
            <Loader2 className="size-3 animate-spin" />
            loading extensions
          </div>
        ) : providerInstallations.length === 0 ? (
          <div className="flex min-h-48 flex-col items-center justify-center gap-2 text-center">
            <PackageOpen className="size-7 text-muted-foreground" strokeWidth={1.25} />
            <div className="font-mono text-[11px] uppercase tracking-widest text-muted-foreground">no extensions found</div>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            {providerInstallations.map((provider) => (
              <ProviderCard
                key={provider.providerId}
                provider={provider}
                toolStatus={toolStatusesById.get(provider.providerId)}
                pending={pendingProviderId === provider.providerId}
                onAction={runAction}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import TopBar from "@/components/windie/TopBar";
import Sidebar from "@/components/windie/Sidebar";
import ChatPanel from "@/components/windie/ChatPanel";
import InspectorPanel from "@/components/windie/InspectorPanel";
import { useWindie } from "@/context/WindieContext";
import { listLlmProviderKeys, listLlmProviders } from "@/lib/windieApi";

export default function Windie() {
  const [overlay, setOverlay] = useState(null);
  const onboardingCheckedRef = useRef(false);
  const [treeCollapsed, setTreeCollapsed] = useState(() => {
    try {
      const value = window.localStorage.getItem("windie.treeCollapsed");
      return value == null ? true : value === "true";
    } catch {
      return true;
    }
  });
  const firstMessageOpenedRef = useRef(false);
  const openTreeForFirstMessage = useCallback(() => {
    if (firstMessageOpenedRef.current) return;
    firstMessageOpenedRef.current = true;
    setTreeCollapsed(false);
  }, []);

  // First-run setup: open onboarding unless Bifrost has validated an enabled
  // provider key. key_count only reports stored key records, so it cannot be
  // used as a readiness signal for auto-detected environment keys.
  useEffect(() => {
    if (onboardingCheckedRef.current) return;
    onboardingCheckedRef.current = true;
    listLlmProviders().then(async (providers) => {
      const readiness = await Promise.all(
        providers.map(async (provider) => {
          if (provider.authentication === "none") {
            return { checked: true, ready: true };
          }
          if (!provider.configured || provider.key_count === 0) {
            return { checked: true, ready: false };
          }

          try {
            const keys = await listLlmProviderKeys(provider.name);
            return {
              checked: true,
              ready: keys.some(
                (key) => key.enabled !== false && key.status === "success"
              ),
            };
          } catch {
            // Keep the existing startup behavior when Bifrost is still coming
            // up or its key endpoint is temporarily unavailable.
            return { checked: false, ready: false };
          }
        })
      );

      if (
        readiness.every(({ checked }) => checked) &&
        !readiness.some(({ ready }) => ready)
      ) {
        setOverlay("onboarding");
      }
    }).catch(() => {
      // The API may still be starting; skipping the auto-show is safer than
      // showing setup to an already-configured user.
    });
  }, []);

  useEffect(() => {
    try {
      window.localStorage.setItem("windie.treeCollapsed", String(treeCollapsed));
    } catch {
      // Storage may be unavailable; panel state still works for this session.
    }
  }, [treeCollapsed]);

  return (
    <div
      data-testid="windie-app-root"
      className="relative h-full w-full flex flex-col bg-background text-foreground overflow-hidden"
    >
      <TopBar
        treeCollapsed={treeCollapsed}
        onTreeToggle={() => setTreeCollapsed((value) => !value)}
        overlay={overlay}
        onOverlayChange={setOverlay}
      />
      <div className="flex-1 min-h-0 flex">
        <Sidebar treeCollapsed={treeCollapsed} />
        <div className="flex-1 min-w-0 relative flex">
          <div className="flex-1 min-w-0 relative flex flex-col min-h-0">
            <ChatPanel onFirstMessage={openTreeForFirstMessage} />
          </div>
          {overlay && <InspectorPanel mode={overlay} onClose={() => setOverlay(null)} />}
        </div>
      </div>
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import TopBar from "@/components/windie/TopBar";
import Sidebar from "@/components/windie/Sidebar";
import ChatPanel from "@/components/windie/ChatPanel";
import InspectorPanel from "@/components/windie/InspectorPanel";
import { useWindie } from "@/context/WindieContext";
import { listLlmProviders } from "@/lib/windieApi";

export default function Windie() {
  const [overlay, setOverlay] = useState(null);
  const onboardingCheckedRef = useRef(false);
  const [treeCollapsed, setTreeCollapsed] = useState(() => {
    try {
      const value = window.localStorage.getItem("windie.treeCollapsed");
      return value == null ? false : value === "true";
    } catch {
      return false;
    }
  });

  // First-run setup: if no LLM provider is configured in Bifrost, open the
  // onboarding overlay once. No persisted flag — configuration state itself is
  // the signal, so CLI-onboarded users never see this.
  useEffect(() => {
    if (onboardingCheckedRef.current) return;
    onboardingCheckedRef.current = true;
    listLlmProviders()
      .then((providers) => {
        const configured = providers.some(
          (provider) => provider.configured || provider.authentication === "none"
        );
        if (!configured) setOverlay("onboarding");
      })
      .catch(() => {
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
            <ChatPanel />
          </div>
          {overlay && <InspectorPanel mode={overlay} onClose={() => setOverlay(null)} />}
        </div>
      </div>
    </div>
  );
}

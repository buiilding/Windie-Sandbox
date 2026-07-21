import { useState } from "react";
import TopBar from "@/components/windie/TopBar";
import Sidebar from "@/components/windie/Sidebar";
import ChatPanel from "@/components/windie/ChatPanel";
import InspectorPanel from "@/components/windie/InspectorPanel";
import { useWindie } from "@/context/WindieContext";

export default function Windie() {
  const { inspectorPanelOpen } = useWindie();
  const [treeCollapsed, setTreeCollapsed] = useState(false);

  return (
    <div
      data-testid="windie-app-root"
      className="h-full w-full flex flex-col bg-background text-foreground overflow-hidden"
    >
      <TopBar treeCollapsed={treeCollapsed} onTreeToggle={() => setTreeCollapsed((value) => !value)} />
      <div className="flex-1 min-h-0 flex">
        <Sidebar treeCollapsed={treeCollapsed} />
        <div className="flex-1 min-w-0 relative flex">
          <div className="flex-1 min-w-0 relative flex flex-col min-h-0">
            <ChatPanel />
          </div>
          {inspectorPanelOpen && <InspectorPanel />}
        </div>
      </div>
    </div>
  );
}

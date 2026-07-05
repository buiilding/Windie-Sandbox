import TopBar from "@/components/windie/TopBar";
import Sidebar from "@/components/windie/Sidebar";
import ChatPanel from "@/components/windie/ChatPanel";
import InspectorPanel from "@/components/windie/InspectorPanel";
import TreeOverlay from "@/components/windie/TreeOverlay";
import { useWindie } from "@/context/WindieContext";

export default function Windie() {
  const { treeOverlayOpen } = useWindie();

  return (
    <div
      data-testid="windie-app-root"
      className="h-full w-full flex flex-col bg-background text-foreground overflow-hidden"
    >
      <TopBar />
      <div className="flex-1 min-h-0 flex">
        <Sidebar />
        <div className="flex-1 min-w-0 relative flex">
        <div className="flex-1 min-w-0 relative flex flex-col min-h-0">
          <ChatPanel />
          {treeOverlayOpen && <TreeOverlay />}
        </div>
          <InspectorPanel />
        </div>
      </div>
    </div>
  );
}

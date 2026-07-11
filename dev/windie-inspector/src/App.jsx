import { Toaster } from "sonner";
import Windie from "@/pages/Windie";
import { WindieProvider } from "@/context/WindieContext";

function App() {
  return (
    <div className="h-full">
      <WindieProvider>
        <Windie />
        <Toaster
          position="bottom-right"
          theme="system"
          toastOptions={{
            style: {
              fontFamily: "IBM Plex Mono, monospace",
              fontSize: "12px",
              borderRadius: "2px",
            },
          }}
        />
      </WindieProvider>
    </div>
  );
}

export default App;

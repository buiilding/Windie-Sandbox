import "@/App.css";
import { BrowserRouter, Routes, Route } from "react-router-dom";
import { Toaster } from "sonner";
import Windie from "@/pages/Windie";
import { WindieProvider } from "@/context/WindieContext";

function App() {
  return (
    <div className="App h-full">
      <WindieProvider>
        <BrowserRouter>
          <Routes>
            <Route path="/" element={<Windie />} />
            <Route path="*" element={<Windie />} />
          </Routes>
        </BrowserRouter>
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

import { useEffect, useState } from "react";

/** Owns browser-only inspector state that is not runtime or persistence state. */
export function useInspectorState() {
  const [theme, setTheme] = useState("light");
  const [contextPreviewOpen, setContextPreviewOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [apiError, setApiError] = useState(null);

  useEffect(() => {
    const root = document.documentElement;
    if (theme === "dark") root.classList.add("dark");
    else root.classList.remove("dark");
  }, [theme]);

  return {
    theme,
    contextPreviewOpen,
    searchQuery,
    apiError,
    setTheme,
    setContextPreviewOpen,
    setSearchQuery,
    setApiError,
  };
}

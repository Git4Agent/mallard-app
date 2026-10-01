import { useEffect, useState } from "react";

export type SessionTheme = "light" | "dark";

function readTheme(): SessionTheme {
  return typeof document !== "undefined" && document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

// Both session viewers stamp the resolved theme on the document root.
export function useSessionTheme(): SessionTheme {
  const [theme, setTheme] = useState(readTheme);
  useEffect(() => {
    const update = () => setTheme(readTheme());
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    update();
    return () => observer.disconnect();
  }, []);
  return theme;
}

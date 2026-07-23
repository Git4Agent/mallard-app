import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import ProfilePagePrototypes from "./components/ProfilePagePrototypes";
import type { AppTheme } from "./types";
import { applyTheme, getStoredTheme } from "./theme";
import "./App.css";

function ProfilePrototypeApp() {
  const [theme, setTheme] = useState<AppTheme>(getStoredTheme);

  useEffect(() => applyTheme(theme), [theme]);

  return <ProfilePagePrototypes theme={theme} onThemeChange={setTheme} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ProfilePrototypeApp />
  </StrictMode>,
);

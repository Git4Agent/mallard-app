import { useEffect, useState } from "react";
import ProjectSync from "./components/project-sync/ProjectSync";
import type { AppTheme } from "./types";
import { applyTheme, getStoredTheme } from "./theme";
import "./App.css";
import "./RetroTheme.css";

export default function App() {
  const [theme, setTheme] = useState<AppTheme>(getStoredTheme);

  useEffect(() => applyTheme(theme), [theme]);

  return <ProjectSync theme={theme} onThemeChange={setTheme} />;
}

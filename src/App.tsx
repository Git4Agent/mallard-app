import { useEffect, useState } from "react";
import ProjectSync from "./components/project-sync/ProjectSync";
import type { AppTheme } from "./types";
import {
  applyResolvedTheme,
  getThemeChoice,
  setThemeChoice,
  subscribePreferences,
  systemTheme,
  type ThemeChoice,
} from "./preferences";
import "./App.css";
import "./RetroTheme.css";

export default function App() {
  const [choice, setChoice] = useState<ThemeChoice>(getThemeChoice);
  const [osTheme, setOsTheme] = useState<AppTheme>(systemTheme);

  useEffect(() => subscribePreferences(() => setChoice(getThemeChoice())), []);

  // Follow the OS while the choice is System; an explicit choice ignores it.
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const follow = () => setOsTheme(media.matches ? "dark" : "light");
    media.addEventListener("change", follow);
    return () => media.removeEventListener("change", follow);
  }, []);

  const theme = choice === "system" ? osTheme : choice;
  useEffect(() => applyResolvedTheme(theme), [theme]);

  // The titlebar toggle stays a light/dark flip: it sets an explicit choice,
  // stepping out of System; the Settings page is where System is picked.
  return <ProjectSync theme={theme} onThemeChange={setThemeChoice} />;
}

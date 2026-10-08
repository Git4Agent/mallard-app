import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyUiLanguage } from "./i18n";
import { applyThemeChoice, getThemeChoice } from "./preferences";

// Apply the saved palette and language before React mounts, so light mode
// does not flash dark and the first frame is already in its language.
applyThemeChoice(getThemeChoice());
applyUiLanguage();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

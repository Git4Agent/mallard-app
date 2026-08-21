import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyThemeChoice, getThemeChoice } from "./preferences";

// Apply the saved palette before React mounts so light mode does not flash dark.
applyThemeChoice(getThemeChoice());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

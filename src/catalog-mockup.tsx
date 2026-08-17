import React from "react";
import ReactDOM from "react-dom/client";
import CatalogRedesignMockup from "./components/CatalogRedesignMockup";
import "./App.css";
import "./RetroTheme.css";
import "./components/CatalogRedesignMockup.css";

document.documentElement.setAttribute("data-theme", "light");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <CatalogRedesignMockup />
  </React.StrictMode>,
);

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyTheme, loadSettings } from "./lib/settings";
import "./index.css";

// Apply the saved theme before first paint to avoid a flash.
applyTheme(loadSettings().theme);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>
);

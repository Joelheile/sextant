import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Popover from "./Popover";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

const label = getCurrentWindow().label;
document.documentElement.classList.toggle("panel", label !== "main");

createRoot(root).render(
  <StrictMode>
    {label === "popover" ? <Popover /> : <App />}
  </StrictMode>,
);

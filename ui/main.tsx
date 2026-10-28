import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import Rewind from "./Rewind";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

const panel = getCurrentWindow().label === "rewind";
document.documentElement.classList.toggle("panel", panel);

createRoot(root).render(
  <StrictMode>
    {panel ? <Rewind /> : <App />}
  </StrictMode>,
);

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { RestartWarning } from "./RestartWarning";
import { QUICK_CREATE_WINDOW } from "./quickCreate";
import "./QuickCreate.css";

if (QUICK_CREATE_WINDOW) document.documentElement.classList.add("quick-create-window");

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
    <RestartWarning />
  </React.StrictMode>,
);

import React from "react";
import ReactDOM from "react-dom/client";

import "./app/styles/theme.css";
import { bootstrap } from "./lib/tauri/bootstrap";

async function start() {
  const { windowLabel } = await bootstrap();

  const Window =
    windowLabel === "settings"
      ? (await import("./windows/SettingsWindow")).default
      : windowLabel === "onboarding"
        ? (await import("./windows/OnboardingWindow")).default
        : (await import("./windows/HudWindow")).default;

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <Window />
    </React.StrictMode>,
  );
}

void start();

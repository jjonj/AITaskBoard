import assert from "node:assert/strict";
import { createServer } from "vite";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

// SSR compiles the actual TSX release component and verifies its accessible
// top-level controls without opening, writing to or restarting a live app.
const server = await createServer({ server: { middlewareMode: true }, appType: "custom" });
try {
  const { ReleaseCenter } = await server.ssrLoadModule("/src/ReleaseCenter.tsx");
  const html = renderToStaticMarkup(React.createElement(ReleaseCenter, {
    boardPath: "temporary-test-board", enabled: false, onClose: () => {}
  }));
  for (const label of ["Releases", "Enable collection", "Import historical Done",
    "Integration needed", "Ready", "Deployed", "Verified", "Failed verification",
    "Close", "Refresh from Markdown"]) {
    assert.ok(html.includes(label), "release UI missing: " + label);
  }
  assert.ok(html.includes('role="dialog"'), "Releases must be its own modal/view");
  assert.ok(html.includes("No releases yet"), "Empty release history should be understandable");
  console.log("ReleaseCenter UI SSR smoke: PASS");
} finally {
  await server.close();
}

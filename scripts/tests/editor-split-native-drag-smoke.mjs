import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/editor-split-native-drag");
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;
const settleWithin = async (promise, timeout) => {
  let timer;
  const result = await Promise.race([
    promise.then((value) => ({ settled: true, value })),
    new Promise((resolve) => { timer = setTimeout(() => resolve({ settled: false }), timeout); }),
  ]);
  clearTimeout(timer);
  return result;
};
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, "index.html"), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, "tauri-core.ts"), "export const invoke = async () => null;");
await writeFile(path.join(fixture, "tauri-opener.ts"), "export const openUrl = async () => undefined;");
await writeFile(
  path.join(fixture, "main.tsx"),
  `
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import { EditorSplitView } from "${source("src/components/ide/EditorSplitView.tsx")}";
import { fileTabKey } from "${source("src/lib/tab-key.ts")}";

const file = (path, dirty = false) => ({ key: fileTabKey(path), path, kind: "text", content: dirty ? "const value = 2;" : "const value = 1;", baseContent: "const value = 1;", mtime: 0, dirty });
function Harness() {
  const [files, setFiles] = useState([file("src/a.ts", true), file("src/b.ts")]);
  const [activeKey, setActiveKey] = useState(fileTabKey("src/b.ts"));
  return <div style={{ display: "flex", height: "600px", width: "1000px" }}><EditorSplitView taskId={1} host="local" windowId="native-drag" files={files} activeKey={activeKey} dark={false} onSelect={setActiveKey} onClose={(key) => setFiles((current) => current.filter((item) => item.key !== key))} onChange={() => undefined} onSave={() => undefined} onReload={() => undefined} onOpenPath={() => undefined} onRevealPath={() => undefined} /></div>;
}
createRoot(document.getElementById("root")).render(<Harness />);
applyTheme("praxis-dark", false);
`,
);

let server;
let browser;
try {
  server = await createServer({
    configFile: false,
    root: fixture,
    plugins: [react()],
    resolve: { alias: [
      { find: "@tauri-apps/api/core", replacement: path.join(fixture, "tauri-core.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: path.join(fixture, "tauri-opener.ts") },
    ] },
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  if (process.env.PRAXIS_SMOKE_SERVE === "1") {
    process.stdout.write(`${server.resolvedUrls.local[0]}index.html\n`);
    await new Promise((resolve) => {
      process.once("SIGINT", resolve);
      process.once("SIGTERM", resolve);
    });
  }
  if (process.env.PRAXIS_SMOKE_SERVE !== "1") {
    const useWebKit = process.env.PRAXIS_SMOKE_BROWSER === "webkit";
    browser = await (useWebKit ? webkit : chromium).launch(
      useWebKit ? { headless: true } : { headless: true, channel: "chrome" },
    );
    const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
    page.setDefaultTimeout(5_000);
    await page.goto(server.resolvedUrls.local[0] + "index.html", {
      waitUntil: "domcontentloaded",
      timeout: 10_000,
    });
    await page.locator(".monaco-editor").waitFor({ timeout: 10_000 });

    const tab = page.locator('[data-tab-path="src/a.ts"]');
    const group = page.locator('[data-group-id="g0"]');
    const [sourceBox, targetBox] = await Promise.all([tab.boundingBox(), group.boundingBox()]);
    assert.ok(sourceBox && targetBox, "native drag targets were not laid out");
    for (const input of [
      () => page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2),
      () => page.mouse.down(),
      () => page.mouse.move(sourceBox.x + sourceBox.width / 2 + 12, sourceBox.y + sourceBox.height / 2),
      () => page.mouse.move(targetBox.x + targetBox.width - 12, targetBox.y + targetBox.height / 2, { steps: 12 }),
      () => page.mouse.up(),
    ]) assert.ok((await settleWithin(input(), 2_000)).settled, "native drag input timed out");

    await page.waitForFunction(
      () => document.querySelectorAll("[data-group-id]").length === 2,
      undefined,
      { timeout: 5_000 },
    );
    assert.equal(await page.locator('[data-group-id="g1"] [data-tab-path="src/a.ts"]').count(), 1);
    assert.equal(await page.locator('[data-group-id="g1"] [aria-label="저장 안 됨"]').count(), 1);
  }
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

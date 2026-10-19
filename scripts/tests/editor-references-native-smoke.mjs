import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/editor-references-native");
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;

await mkdir(fixture, { recursive: true });
await writeFile(
  path.join(fixture, "index.html"),
  '<div id="root"></div><script type="module" src="./main.tsx"></script>',
);
await writeFile(
  path.join(fixture, "tauri-core.ts"),
  "export const invoke = async () => null;",
);
await writeFile(
  path.join(fixture, "main.tsx"),
  `
import React from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import { EditorPane } from "${source("src/components/ide/EditorPane.tsx")}";
import { fileTabKey } from "${source("src/lib/tab-key.ts")}";

const target = { path: "src/target.ts", abs_path: "/workspace/src/target.ts", line: 7, column: 3, external: false };
function Harness() {
  const content = "const unsavedReference = true;";
  const file = { key: fileTabKey("src/main.ts"), path: "src/main.ts", kind: "text", content, baseContent: "const saved = true;", mtime: 0, dirty: true };
  window.__gotoRequests = window.__gotoRequests ?? [];
  window.__navigations = window.__navigations ?? [];
  window.__opened = window.__opened ?? [];
  return <div style={{ display: "flex", height: "600px", width: "1000px" }} data-buffer={content}>
    <EditorPane
      taskId={1}
      files={[file]}
      activeKey={file.key}
      retainedPaths={[file.path]}
      dark={false}
      onSelect={() => undefined}
      onClose={() => undefined}
      onChange={() => undefined}
      onSave={() => undefined}
      onReload={() => undefined}
      onOpenPath={() => undefined}
      onRevealPath={() => undefined}
      onGoto={async (request) => { window.__gotoRequests.push(request); return [target]; }}
      onOpenTarget={async (value) => { window.__opened.push(value); return "opened"; }}
      onNavigateTarget={(value, origin) => window.__navigations.push({ value, origin })}
    />
  </div>;
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
    resolve: {
      alias: [
        { find: "@tauri-apps/api/core", replacement: path.join(fixture, "tauri-core.ts") },
      ],
    },
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  const useWebKit = process.env.PRAXIS_SMOKE_BROWSER === "webkit";
  browser = await (useWebKit ? webkit : chromium).launch(
    useWebKit
      ? { headless: true }
      : { headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" },
  );
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  page.setDefaultTimeout(5_000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.goto(server.resolvedUrls.local[0] + "index.html", {
    waitUntil: "domcontentloaded",
    timeout: 10_000,
  });
  const editor = page.locator(".monaco-editor");
  try {
    await editor.waitFor({ timeout: 10_000 });
  } catch (cause) {
    throw new Error(`${cause}\n${errors.join("\n")}\n${await page.locator("body").innerText()}`);
  }

  const buffer = "const unsavedReference = true;";
  await editor.click();

  await page.keyboard.press("Alt+b");
  await page.waitForFunction(() => window.__gotoRequests.length === 1);
  const request = await page.evaluate(() => window.__gotoRequests[0]);
  assert.deepEqual(request, {
    kind: "references",
    path: "src/main.ts",
    text: buffer,
    line: 1,
    column: buffer.length + 1,
  });
  assert.equal(await page.locator("[data-buffer]").getAttribute("data-buffer"), buffer);

  const references = page.getByRole("dialog", { name: "사용처" });
  await references.waitFor();
  await page.waitForFunction(() => document.activeElement?.getAttribute("role") === "listbox");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => window.__navigations.length === 1);
  await references.getByRole("option", { name: /src\/target.ts:7/ }).click();
  await page.waitForFunction(() => window.__navigations.length === 2);

  const navigations = await page.evaluate(() => window.__navigations);
  assert.equal(navigations[0].value.path, "src/target.ts");
  assert.equal(navigations[0].origin.line, 1);
  assert.equal(navigations[0].origin.column, buffer.length + 1);
  assert.equal(await page.evaluate(() => window.__opened.length), 0);
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

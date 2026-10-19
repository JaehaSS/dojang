import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/tap-tap-editor-smoke");
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, "index.html"), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(
  path.join(fixture, "ipc.ts"),
  `export const projectSearch = async () => ({ matches: [{ path: "src/needle.ts", line: 3, column: 5, text: "const needle = 1;" }], truncated: false }); export const quickopenSearch = async () => []; export const skillsList = async () => [];`,
);
await writeFile(path.join(fixture, "host-scope.ts"), 'export const useHostScope = () => "local";');
await writeFile(
  path.join(fixture, "main.tsx"),
  `
import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import Editor from "@monaco-editor/react";
import { onKeyDown, onKeyUp } from "${source("src/lib/shift-double-tap.ts")}";
import { QuickOpen } from "${source("src/components/QuickOpen.tsx")}";

function Harness() {
  const [fires, setFires] = useState(0);
  const [open, setOpen] = useState(false);
  const [reveal, setReveal] = useState("");
  const editorRef = useRef(null);
  const capture = ${process.env.PRAXIS_TAP_CAPTURE !== "0"};
  useEffect(() => {
    let tap = { lastUp: 0 };
    window.__tapEvents = [];
    const down = (event) => { window.__tapEvents.push(["down", event.key, performance.now(), event.isComposing]); tap = onKeyDown(tap, event); };
    const up = (event) => {
      window.__tapEvents.push(["up", event.key, performance.now(), event.isComposing]);
      const result = onKeyUp(tap, event, performance.now());
      tap = result.next;
      if (result.fire) { setFires((value) => value + 1); setOpen(true); }
    };
    window.addEventListener("keydown", down, capture);
    window.addEventListener("keyup", up, capture);
    return () => {
      window.removeEventListener("keydown", down, capture);
      window.removeEventListener("keyup", up, capture);
    };
  }, [capture]);
  const onSelect = (item) => {
    setReveal(item.id);
    if (item.scope === "code") window.__editor.revealPosition({ lineNumber: 3, column: 5 });
    setOpen(false);
  };
  return <><Editor height="240px" defaultLanguage="typescript" defaultValue="const dirty = true;" onMount={(editor) => { editorRef.current = editor; window.__editor = editor; editor.focus(); }} /><div data-fires={fires}>{fires}</div><div data-reveal={reveal} /><QuickOpen open={open} editorSearch={{ scopeLabel: "feature/search · local · 세션 #42", contentAvailable: true }} scopes={["file", "code"]} files={["src/alpha.ts"]} repo="" taskId={42} onClose={() => setOpen(false)} onSelect={onSelect} /></>;
}
createRoot(document.getElementById("root")).render(<Harness />);
applyTheme("praxis-dark", false);
window.__applyTheme = applyTheme;
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
      { find: "../lib/ipc", replacement: path.join(fixture, "ipc.ts") },
      { find: "../lib/host-scope", replacement: path.join(fixture, "host-scope.ts") },
    ] },
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  const useWebKit = process.env.PRAXIS_SMOKE_BROWSER === "webkit";
  browser = await (useWebKit ? webkit : chromium).launch(
    useWebKit ? { headless: true } : { headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" },
  );
  const page = await browser.newPage();
  await page.goto(server.resolvedUrls.local[0] + "index.html");
  const editor = page.locator(".monaco-editor");
  await editor.waitFor();
  await page.waitForFunction(() => window.__editor != null);
  const tap = async (expected) => {
    await page.evaluate(async () => {
      const target = window.__editor.getDomNode();
      const shift = (type) => target.dispatchEvent(new KeyboardEvent(type, { key: "Shift", bubbles: true }));
      shift("keydown"); shift("keyup");
      await new Promise((resolve) => setTimeout(resolve, 20));
      shift("keydown"); shift("keyup");
    });
    try {
      await page.waitForFunction((count) => document.querySelector("[data-fires]")?.getAttribute("data-fires") === String(count), expected, { timeout: 1000 });
    } catch {
      throw new Error(JSON.stringify(await page.evaluate(() => window.__tapEvents)));
    }
  };
  await tap(1);
  const search = page.getByRole("dialog", { name: "빠른 검색" });
  await search.waitFor();
  const searchInput = search.getByLabel("빠른 검색");
  await search.getByRole("tab", { name: "파일" }).click();
  await page.keyboard.type("needle");
  await searchInput.press("Tab");
  await searchInput.press("Shift+Tab");
  await searchInput.press("Tab");
  await search.getByRole("button", { name: "src/needle.ts:3" }).waitFor();
  await page.screenshot({ path: "/tmp/praxis-editor-search-dark.png", fullPage: true });
  await page.evaluate(() => window.__applyTheme("praxis-light", false));
  await page.screenshot({ path: "/tmp/praxis-editor-search-light.png", fullPage: true });
  await searchInput.press("Enter");
  await page.waitForFunction(() => document.querySelector("[data-reveal]")?.getAttribute("data-reveal") === "src/needle.ts:3:5");
  assert.equal(await page.evaluate(() => window.__editor.getValue()), "const dirty = true;");
  assert.equal(await page.evaluate(() => window.__editor.hasTextFocus()), true);
  await page.evaluate(() => window.__editor.getAction("actions.find")?.run());
  const findInput = page.locator(".find-widget textarea").first();
  await findInput.waitFor();
  await findInput.focus();
  await tap(2);
  await page.getByRole("dialog", { name: "빠른 검색" }).getByLabel("빠른 검색").press("Escape");
  await page.evaluate(() => window.__editor.focus());
  await page.evaluate(() => window.__editor.trigger("smoke", "editor.action.triggerSuggest"));
  await page.locator(".suggest-widget.visible").waitFor();
  await tap(3);
  await page.getByRole("dialog", { name: "빠른 검색" }).getByLabel("빠른 검색").press("Escape");
  await page.keyboard.press("Shift+A");
  assert.equal(await page.locator("[data-fires]").getAttribute("data-fires"), "3");
  await editor.evaluate((node) => {
    node.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift", bubbles: true, isComposing: true }));
    node.dispatchEvent(new KeyboardEvent("keyup", { key: "Shift", bubbles: true, isComposing: true }));
  });
  assert.equal(await page.locator("[data-fires]").getAttribute("data-fires"), "3");
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

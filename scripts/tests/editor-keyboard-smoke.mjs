import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const run = promisify(execFile);
await mkdir(path.resolve(".dojang/verification"), { recursive: true });
const directory = await mkdtemp(path.resolve(".dojang/verification/editor-keyboard-"));
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;
let server;
let browser;

try {
  await mkdir(fixture, { recursive: true });
  await writeFile(path.join(fixture, "index.html"), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
  await writeFile(path.join(fixture, "plain.html"), '<input id="plain-input" value="plain input text"><textarea id="plain-textarea">plain textarea text</textarea><script>window.__keyboard = { ready: () => true };</script>');
  await writeFile(path.join(fixture, "core.ts"), "export const invoke = async () => null;");
  await writeFile(path.join(fixture, "opener.ts"), "export const openUrl = async () => undefined;");
  await writeFile(path.join(fixture, "main.tsx"), `
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import { monaco } from "${source("src/lib/monaco.ts")}";
import { EditorSplitView } from "${source("src/components/ide/EditorSplitView.tsx")}";
import { FileTree } from "${source("src/components/ide/FileTree.tsx")}";
import { fileTabKey } from "${source("src/lib/tab-key.ts")}";

// Longer than Monaco's hidden textarea page: selecting that page is not select-all.
const text = Array.from({ length: 120 }, (_, i) => "const line" + i + " = " + i + ";").join("\\n");
const file = (path) => ({ key: fileTabKey(path), path, kind: "text", content: text, baseContent: text, dirty: false, mtime: 0 });
window.__keyboard = {
  original: text, saves: [],
  ready: () => monaco.editor.getEditors().length > 0,
  editor: () => monaco.editor.getEditors().find((ed) => ed.getModel()?.uri.toString().includes(window.__keyboard.path)),
  snapshot: () => {
    const ed = window.__keyboard.editor();
    return { content: ed.getValue(), selected: ed.getModel().getValueInRange(ed.getSelection()) };
  },
  focus: () => window.__keyboard.editor().focus(),
  open: (mode) => document.querySelector('[data-open="' + mode + '"]').click(),
};
function Harness() {
  const [files, setFiles] = useState([file("zoom.txt")]);
  const [activeKey, setActiveKey] = useState(fileTabKey("zoom.txt"));
  window.__keyboard.path = activeKey === fileTabKey("tree.txt") ? "tree.txt" : "zoom.txt";
  const open = (path) => { setFiles((current) => current.some((f) => f.path === path) ? current : [...current, file(path)]); setActiveKey(fileTabKey(path)); };
  return <>
    <button data-open="zoom" onClick={() => open("zoom.txt")}>줌아웃 파일 열기</button>
    <input id="plain-input" defaultValue="plain input text" />
    <textarea id="plain-textarea" defaultValue="plain textarea text" />
    <div style={{ display: "flex", height: "600px", width: "1000px" }}>
      <div style={{ width: "180px" }} data-open="tree" onClick={() => open("tree.txt")}>
        <FileTree nodes={[{ name: "tree.txt", path: "tree.txt", is_dir: false, children: [] }]} activePath={window.__keyboard.path} onOpen={open} />
      </div>
      <EditorSplitView taskId={1} host="local" windowId="keyboard-smoke" files={files} activeKey={activeKey} dark={false}
        onSelect={setActiveKey} onClose={() => undefined} onChange={(path, content) => setFiles((current) => current.map((f) => f.path === path ? { ...f, content, dirty: true } : f))}
        onSave={(path, content) => window.__keyboard.saves.push({ path, content })} onReload={() => undefined} onOpenPath={() => undefined} onRevealPath={() => undefined} />
    </div>
  </>;
}
createRoot(document.getElementById("root")).render(<Harness />);
`);
  server = await createServer({
    configFile: false, root: fixture, plugins: [react()],
    resolve: { alias: [
      { find: "@tauri-apps/api/core", replacement: path.join(fixture, "core.ts") },
      { find: "@tauri-apps/plugin-opener", replacement: path.join(fixture, "opener.ts") },
    ] },
    server: { host: "127.0.0.1", port: 0 }, logLevel: "error",
  });
  await server.listen();
  const url = server.resolvedUrls.local[0] + "index.html";
  const useWebKit = process.env.PRAXIS_SMOKE_BROWSER === "webkit";
  browser = await (useWebKit ? webkit : chromium).launch(useWebKit ? { headless: true } : { headless: true, channel: "chrome" });
  const page = await browser.newPage();
  page.setDefaultTimeout(10_000);
  const errors = [];
  page.on("pageerror", (error) => {
    // Rapid edits/tab switches cancel Monaco's pending asynchronous requests.
    if (error.message !== "Canceled") errors.push(error.message);
  });
  await page.goto(url);
  await page.waitForFunction(() => window.__keyboard?.ready());
  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  for (const mode of ["zoom", "tree"]) {
    await page.locator(mode === "tree" ? '[role="treeitem"]' : '[data-open="zoom"]').click();
    await page.waitForFunction((mode) => !!window.__keyboard.editor() && window.__keyboard.path === mode + ".txt", mode);
    await page.evaluate(() => window.__keyboard.focus());
    const original = await page.evaluate(() => window.__keyboard.original);
    await page.keyboard.press(modifier + "+a");
    assert.equal((await page.evaluate(() => window.__keyboard.snapshot())).selected, original, mode + " selects the whole document");
    await page.keyboard.insertText("replacement");
    await page.keyboard.press(modifier + "+z");
    assert.equal((await page.evaluate(() => window.__keyboard.snapshot())).content, original, mode + " undo");
    await page.keyboard.press(modifier + "+Shift+z");
    assert.equal((await page.evaluate(() => window.__keyboard.snapshot())).content, "replacement", mode + " redo");
    await page.keyboard.press(modifier + "+s");
    assert.deepEqual(await page.evaluate(() => window.__keyboard.saves.at(-1)), { path: mode + ".txt", content: "replacement" });
    await page.keyboard.press(modifier + "+z");
    console.log(`PASS ${useWebKit ? "WebKit" : "Chromium"} ${mode}: select-all, replace, undo, redo, save`);
  }
  for (const id of ["plain-input", "plain-textarea"]) {
    await page.locator("#" + id).focus();
    await page.keyboard.press(modifier + "+a");
    assert.equal(await page.locator("#" + id).evaluate((el) => el.value.slice(el.selectionStart, el.selectionEnd)), await page.locator("#" + id).inputValue());
  }
  assert.deepEqual(errors, []);

  if (process.platform === "darwin") {
    // Use the actual app menu configuration. The pre-fix build must fail here.
    const rust = await readFile("src-tauri/src/lib.rs", "utf8");
    const edit = rust.match(/let edit_menu = SubmenuBuilder[\s\S]*?\.build\(\)\?;/)?.[0];
    assert.ok(edit, "macOS Edit menu configuration not found");
    const executable = path.join(directory, "native-keyboard");
    await run("swiftc", ["scripts/tests/editor-keyboard-native.swift", "-o", executable], { timeout: 60_000 });
    const dispatch = rust.match(/webview\.eval\(\s*"([^"]*dojang:select-all[^"]*)"/)?.[1];
    assert.ok(dispatch, "native select-all must target the focused webview");
    assert.ok(edit.includes(".item(&select_all)"), "native Select All must use the custom menu item");
    const menuMode = process.env.PRAXIS_SMOKE_MENU || (edit.includes(".select_all()") ? "predefined" : "custom");
    const { stdout } = await run(executable, [url, menuMode, dispatch], { timeout: 30_000 });
    const result = JSON.parse(stdout.trim());
    console.log("Native selection lengths:", result.selections.map(({ mode, selected, expected }) => ({ mode, selected: selected.length, expected: expected.length })));
    for (const entry of result.selections) assert.ok(entry.selected === entry.expected, `native WKWebView ${entry.mode}: selected ${entry.selected.length} of ${entry.expected.length} characters`);
    assert.equal(result.menuCaptured, true, "native Select All accelerator must be exercised");
    console.log("PASS native AppKit/WKWebView: Command+A reaches both file entry paths, input and textarea");
    const plain = await run(executable, [server.resolvedUrls.local[0] + "plain.html", "custom", dispatch, "plain-input,plain-textarea"], { timeout: 30_000 });
    for (const entry of JSON.parse(plain.stdout.trim()).selections) assert.equal(entry.selected, entry.expected, `native WKWebView without Monaco: ${entry.mode}`);
    console.log("PASS native AppKit/WKWebView: DOM fallback in windows without Monaco");
  }
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

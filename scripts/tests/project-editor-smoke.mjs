import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";

import react from "@vitejs/plugin-react";
import { chromium } from "playwright";
import { createServer } from "vite";

const directory = path.resolve(".dojang/verification/project-editor-smoke");
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;
const screenshots = [];
let browser;
let server;

await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, "index.html"), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, "event.ts"), `
const listeners = new Map();
export const emit = async (...args) => { window.__native.emits.push(args); };
export const listen = async (name, callback) => {
  const values = listeners.get(name) ?? [];
  values.push(callback); listeners.set(name, values);
  return () => listeners.set(name, (listeners.get(name) ?? []).filter((item) => item !== callback));
};
window.__emitNative = (name, payload) => (listeners.get(name) ?? []).forEach((callback) => callback({ payload }));
`);
await writeFile(path.join(fixture, "window.ts"), `
export const getCurrentWindow = () => ({
  onCloseRequested: async (callback) => { window.__native.closeRequested = callback; return () => { if (window.__native.closeRequested === callback) window.__native.closeRequested = null; }; },
  close: async () => { window.__native.closeCalls += 1; },
});
window.__requestNativeClose = () => {
  let prevented = false;
  window.__native.closeRequested?.({ preventDefault: () => { prevented = true; } });
  return prevented;
};
`);
await writeFile(path.join(fixture, "dialog.ts"), `
export const open = async () => window.__native.dialogResponses.shift() ?? null;
`);
await writeFile(path.join(fixture, "opener.ts"), `
export const openUrl = async () => undefined;
export const revealItemInDir = async () => undefined;
`);
await writeFile(path.join(fixture, "core.ts"), `
export const invoke = async (command) => {
  window.__native.nativeCommands.push(command);
  if (command === "editor_settings_get") return { tree_font_size: 16, minimap: false, word_wrap: false, tab_size: 2 };
  if (command === "font_settings_get") return { ui_family: "", code_family: "", ui_size: 13, code_size: 13 };
  throw new Error("unexpected non-project IPC: " + command);
};
`);
await writeFile(path.join(fixture, "project-editor-ipc.ts"), `
const b64 = (text) => btoa(unescape(encodeURIComponent(text)));
const state = window.__native;
const command = (name) => state.nativeCommands.push(name);
export const projectEditorInfo = async () => { command("project_editor_info"); return { root: "/canonical/original-root", label: "project-editor-smoke" }; };
export const projectEditorTree = async () => { command("project_editor_tree"); return [{ name: "src", path: "src", is_dir: true, children: [{ name: "main.ts", path: "src/main.ts", is_dir: false, children: [] }] }, { name: "README.md", path: "README.md", is_dir: false, children: [] }]; };
export const projectEditorRead = async (path) => {
  command("project_editor_read");
  if (!(path in state.files)) throw new Error("missing fixture file: " + path);
  return { kind: "text", content: state.files[path], mtime: state.mtimes[path] };
};
export const projectEditorWrite = async (path, content, expectedContent) => {
  command("project_editor_write");
  state.calls.push(["write", path, content, expectedContent]);
  if (state.files[path] !== expectedContent) throw new Error("external content changed");
  state.files[path] = content; state.mtimes[path] += 1;
  return state.mtimes[path];
};
export const projectEditorResolvePath = async (path) => { command("project_editor_resolve_path"); return "/canonical/original-root/" + path; };
export const projectEditorOpenPath = async () => { command("project_editor_open_path"); };
export const projectEditorOpen = async (root) => {
  command("project_editor_open");
  state.calls.push(["open", root]);
  if (root === "/workspace/bad") throw new Error("directory access denied");
  return { root: "/canonical" + root, label: "project-editor-" + state.calls.length };
};
export const projectShellOpen = async () => {
  command("project_editor_shell_open");
  const session = ++state.shellSessions;
  state.calls.push(["shell_open", session]);
  return { session, existed: false };
};
export const projectShellSnapshot = async (session) => {
  command("project_editor_shell_snapshot");
  state.calls.push(["shell_snapshot", session]);
  return new Promise((resolve) => { state.snapshotResolvers[session] = () => resolve({ session, sequence: 3, data: b64("snapshot session " + session + "\\r\\n"), exited: false, exit_code: null }); });
};
export const projectShellWrite = async () => { command("project_editor_shell_write"); };
export const projectShellResize = async () => { command("project_editor_shell_resize"); };
export const projectShellClose = async (session) => { command("project_editor_shell_close"); state.calls.push(["shell_close", session]); };
`);
await writeFile(path.join(fixture, "main.tsx"), `
import React from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import { HomeProjectEditor } from "${source("src/components/ide/HomeProjectEditor.tsx")}";
import { ProjectEditorWindow } from "${source("src/components/ide/ProjectEditorWindow.tsx")}";
import { projectEditorOpen } from "./project-editor-ipc";

function Harness() {
  if (new URLSearchParams(location.search).get("mode") === "home") return <main className="min-h-screen bg-bg px-3 pt-3 text-text"><HomeProjectEditor localRoots={["/workspace/local-one", "/workspace/local-two", "/workspace/bad"]} onOpen={async (root) => (await projectEditorOpen(root)).root} /></main>;
  return <ProjectEditorWindow />;
}
applyTheme("praxis-dark", false);
window.__applyTheme = (theme) => applyTheme(theme, false);
createRoot(document.getElementById("root")).render(<Harness />);
`);

try {
  server = await createServer({
    configFile: false,
    root: fixture,
    plugins: [{
      name: "project-editor-native-fixture",
      enforce: "pre",
      resolveId(id) {
        if (id === "@tauri-apps/api/event") return path.join(fixture, "event.ts");
        if (id === "@tauri-apps/api/window") return path.join(fixture, "window.ts");
        if (id === "@tauri-apps/api/core") return path.join(fixture, "core.ts");
        if (id === "@tauri-apps/plugin-dialog") return path.join(fixture, "dialog.ts");
        if (id === "@tauri-apps/plugin-opener") return path.join(fixture, "opener.ts");
        if (id.endsWith("/project-editor-ipc")) return path.join(fixture, "project-editor-ipc.ts");
      },
    }, react()],
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 1360, height: 940 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript(() => {
    window.__native = {
      calls: [], nativeCommands: [], emits: [], closeCalls: 0, closeRequested: null,
      dialogResponses: [null, "/workspace/picked"],
      confirmResponses: [false, true],
      files: { "src/main.ts": 'export const source = "original";\n', "README.md": "# Original root\n" },
      mtimes: { "src/main.ts": 1, "README.md": 1 }, shellSessions: 0, snapshotResolvers: {},
    };
    window.confirm = () => window.__native.confirmResponses.shift() ?? true;
    localStorage.setItem("praxis-project-editor-recent", JSON.stringify([{ path: "/recent/alpha", lastUsed: 100 }]));
  });
  await page.goto(server.resolvedUrls.local[0] + "index.html?mode=home");
  const shot = async (name) => {
    const file = `/tmp/home-project-editor-${name}.png`;
    await page.screenshot({ path: file, fullPage: true });
    screenshots.push(file);
  };
  const openCalls = async () => page.evaluate(() => window.__native.calls.filter(([name]) => name === "open"));
  const xtermCanvas = () => page.locator(".xterm-screen canvas").last().evaluate((canvas) => canvas.toDataURL());
  const assertNoHorizontalOverflow = async (width) => {
    const initialDocumentWidth = await page.evaluate(() => document.documentElement.scrollWidth);
    try {
      await page.waitForFunction(async () => {
        if (document.documentElement.scrollWidth > innerWidth) return false;
        await new Promise(requestAnimationFrame);
        return document.documentElement.scrollWidth <= innerWidth;
      }, undefined, { timeout: 3_000 });
      if (initialDocumentWidth > width) console.log(`layout settled after resize: ${width}px initial scrollWidth=${initialDocumentWidth}`);
    } catch {
      const screenshot = `/tmp/home-project-editor-overflow-${width}.png`;
      await page.screenshot({ path: screenshot, fullPage: true });
      const metrics = await page.evaluate(() => ({
        viewport: innerWidth,
        documentWidth: document.documentElement.scrollWidth,
        offenders: [...document.querySelectorAll("*")]
          .map((element) => {
            const rect = element.getBoundingClientRect();
            return { tag: element.tagName, className: element.className, testId: element.getAttribute("data-testid"), path: element.getAttribute("data-tab-path"), left: Math.round(rect.left), right: Math.round(rect.right), width: Math.round(rect.width) };
          })
          .filter((item) => item.right > innerWidth + 1 || item.left < -1)
          .slice(0, 24),
      }));
      throw new Error(`${width}px layout overflow: ${JSON.stringify(metrics)}; screenshot: ${screenshot}`);
    }
  };

  await page.getByRole("button", { name: "에디터 열기" }).click();
  await page.getByRole("button", { name: "폴더 열기…" }).click();
  assert.equal((await openCalls()).length, 0, "cancelled native folder selection must not open a project");

  await page.getByRole("button", { name: "에디터 열기" }).click();
  await page.getByRole("button", { name: "폴더 열기…" }).click();
  await page.waitForFunction(() => window.__native.calls.some(([name, root]) => name === "open" && root === "/workspace/picked"));
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem("praxis-project-editor-recent") ?? "[]")[0]?.path), "/canonical/workspace/picked");

  await page.getByRole("button", { name: "에디터 열기" }).click();
  await page.getByLabel("레포 검색").fill("bad");
  await page.getByLabel("레포 검색").press("Enter");
  await page.getByRole("alert").getByText("directory access denied").waitFor();
  assert.equal(await page.evaluate(() => window.__native.nativeCommands.some((name) => /task_create|worktree/.test(name))), false, "home project opening must not invoke task or worktree commands");

  await page.goto(server.resolvedUrls.local[0] + "index.html?mode=project");

  const srcDirectory = page.getByRole("treeitem", { name: "src", exact: true });
  if (await srcDirectory.getAttribute("aria-expanded") === "false") await srcDirectory.click();
  await page.getByTitle("src/main.ts", { exact: true }).click();
  const editor = page.locator(".monaco-editor");
  await editor.waitFor();
  await editor.click();
  await page.keyboard.press("Meta+A");
  await page.keyboard.type('export const source = "saved from monaco";');
  await page.keyboard.press("Meta+S");
  await page.waitForFunction(() => window.__native.files["src/main.ts"] === 'export const source = "saved from monaco";');
  assert.equal((await page.evaluate(() => window.__native.calls.filter(([name]) => name === "write"))).length, 1, "Monaco save must call the root-bound write IPC once");

  await editor.click();
  await page.keyboard.press("End");
  await page.keyboard.type(" // unsaved");
  await page.locator('[data-tab-path="src/main.ts"] [aria-label="닫기"]').click();
  assert.equal(await page.locator('[data-tab-path="src/main.ts"]').count(), 1, "cancelling a dirty close must retain the tab");
  assert.equal(await page.locator('[data-tab-path="src/main.ts"] [aria-label="저장 안 됨"]').count(), 1, "cancelled close must retain dirty state");
  await editor.click();
  await page.keyboard.press("Meta+S");
  await page.waitForFunction(() => window.__native.files["src/main.ts"].endsWith(" // unsaved"));

  await page.getByRole("button", { name: "터미널 열기", exact: true }).click();
  await page.locator("[data-project-shell]").waitFor();
  await page.waitForFunction(() => typeof window.__native.snapshotResolvers[1] === "function");
  const emptyTerminal = await xtermCanvas();
  await page.evaluate(() => window.__native.snapshotResolvers[1]());
  await page.waitForFunction((before) => document.querySelector(".xterm-screen canvas:last-child")?.toDataURL() !== before, emptyTerminal);
  const snapshotTerminal = await xtermCanvas();
  await page.evaluate(() => window.__emitNative("project-shell://output", { session: 1, sequence: 4, data: btoa("live output\\r\\n") }));
  await page.waitForFunction((before) => document.querySelector(".xterm-screen canvas:last-child")?.toDataURL() !== before, snapshotTerminal);
  const liveTerminal = await xtermCanvas();
  await page.getByRole("button", { name: "터미널 닫기", exact: true }).click();
  assert.equal(await page.locator("[data-project-shell]").count(), 1, "collapsing must retain the mounted project terminal");
  assert.equal(await xtermCanvas(), liveTerminal, "collapse must retain the rendered xterm scrollback");
  await page.evaluate(() => window.__emitNative("project-shell://output", { session: 1, sequence: 5, data: btoa("while collapsed\\r\\n") }));
  await page.getByRole("button", { name: "터미널 열기", exact: true }).click();
  await page.waitForFunction((before) => document.querySelector(".xterm-screen canvas:last-child")?.toDataURL() !== before, liveTerminal);
  await page.evaluate(() => window.__emitNative("project-shell://exit", { session: 1, code: 0 }));
  await page.getByRole("button", { name: "재시작" }).click();
  await page.waitForFunction(() => window.__native.calls.some(([name, session]) => name === "shell_close" && session === 1));
  await page.waitForFunction(() => typeof window.__native.snapshotResolvers[2] === "function");
  const restartedEmptyTerminal = await xtermCanvas();
  await page.evaluate(() => window.__native.snapshotResolvers[2]());
  await page.waitForFunction((before) => document.querySelector(".xterm-screen canvas:last-child")?.toDataURL() !== before, restartedEmptyTerminal);

  await shot("dark");
  await page.evaluate(() => window.__applyTheme("praxis-light"));
  await page.waitForFunction(() => document.documentElement.dataset.theme === "praxis-light");
  await shot("light");
  await page.setViewportSize({ width: 1100, height: 900 });
  await assertNoHorizontalOverflow(1100);
  await shot("1100");
  await page.setViewportSize({ width: 720, height: 900 });
  await assertNoHorizontalOverflow(720);
  await shot("720");

  assert.equal(await page.evaluate(() => window.__requestNativeClose()), true, "fixture must deliver the Tauri close callback and prevent the initial close");
  await page.waitForFunction(() => window.__native.closeCalls === 1);
  assert.deepEqual(errors, [], `browser errors: ${errors.join("\n")}`);
  assert.equal(await page.evaluate(() => window.__native.nativeCommands.some((name) => /task_create|worktree/.test(name))), false, "the project editor must not invoke task or worktree commands");
  console.log("PASS: HomeProjectEditor and ProjectEditorWindow exercised with real Monaco and xterm: recent search, folder cancellation, canonical open, error, root save, dirty-close cancel, terminal snapshot/live/collapse/restart, theme, narrow layouts, and window-close callback.");
  console.log(screenshots.join("\n"));
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

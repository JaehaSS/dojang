import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/editor-links-preview");
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;

let server;
let browser;
try {
  await mkdir(fixture, { recursive: true });
  await writeFile(
    path.join(fixture, "index.html"),
    '<!doctype html><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>',
  );
  await writeFile(path.join(fixture, "tauri-core.ts"), "export const invoke = async () => null;");
  await writeFile(path.join(fixture, "tauri-opener.ts"), "export const openUrl = async () => undefined;");
  await writeFile(
    path.join(fixture, "main.tsx"),
    `
import React, { useEffect } from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import type { FsNode } from "${source("src/lib/ipc.ts")}";
import { EditorSplitView } from "${source("src/components/ide/EditorSplitView.tsx")}";
import { FileTree } from "${source("src/components/ide/FileTree.tsx")}";
import { useWorkspaceFiles, type WorkspaceFileSource } from "${source("src/components/ide/useWorkspaceFiles.ts")}";
import { fileTabKey } from "${source("src/lib/tab-key.ts")}";

const contents: Record<string, string> = {
  "docs/source.md": "# Source\\n\\n[Open target](target.md)",
  "docs/target.md": "# Target",
  "docs/preview-a.md": "# Preview A",
  "docs/preview-b.md": "# Preview B",
};
const tree: FsNode[] = [{
  name: "docs",
  path: "docs",
  is_dir: true,
  children: Object.keys(contents).map((file) => ({
    name: file.slice("docs/".length), path: file, is_dir: false, children: [],
  })),
}];
const workspace: WorkspaceFileSource = {
  key: "smoke:editor-links-preview",
  tree: async () => tree,
  read: async (file) => {
    const content = contents[file];
    if (content == null) throw new Error("missing fixture file: " + file);
    return { kind: "text", content, mtime: 1 };
  },
  write: async () => 2,
};

function Harness() {
  const files = useWorkspaceFiles({
    task: null,
    source: workspace,
    onError: (message) => { throw new Error("workspace fixture error: " + message); },
  });
  useEffect(() => {
    files.refreshTree();
    void files.openFile("docs/source.md");
  }, [files.openFile, files.refreshTree]);
  return <main style={{ display: "flex", height: "700px", width: "1200px" }}>
    <aside style={{ width: "260px" }}>
      <FileTree
        nodes={files.tree}
        activePath={files.activeFile?.path ?? null}
        onOpen={(file) => void files.openFile(file, { preview: true, tree: true })}
        onPin={(file) => files.pinTab(fileTabKey(file))}
      />
    </aside>
    <EditorSplitView
      taskId={null}
      host="local"
      windowId="editor-links-preview-smoke"
      ownsWindow
      files={files.openFiles}
      activeKey={files.activeKey}
      treeOpen={files.treeOpen}
      onTreeOpenHandled={files.consumeTreeOpen}
      dark={false}
      onSelect={files.setActiveKey}
      onClose={files.closeTab}
      onOpenFile={files.openFile}
      onPinTab={files.pinTab}
      onChange={files.changeFile}
      onSave={(file, content) => void files.saveFile(file, content)}
      onReload={(file) => void files.reloadFile(file)}
      onOpenPath={() => undefined}
      onRevealPath={() => undefined}
    />
  </main>;
}

if (new URLSearchParams(location.search).get("preview") === "off") {
  localStorage.setItem("praxis:preview-tabs", "off");
}
createRoot(document.getElementById("root")!).render(<Harness />);
applyTheme("praxis-dark", false);
`,
  );

  server = await createServer({
    configFile: false,
    root: fixture,
    plugins: [react()],
    resolve: {
      alias: [
        { find: "@tauri-apps/api/core", replacement: path.join(fixture, "tauri-core.ts") },
        { find: "@tauri-apps/plugin-opener", replacement: path.join(fixture, "tauri-opener.ts") },
      ],
    },
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: "chrome" });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
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

  const pane = (id) => page.locator(`[data-group-id="${id}"]`);
  const tab = (id, file) => pane(id).locator(`[data-tab-path="${file}"]`);
  const tabPaths = async (id) => pane(id).locator("[data-tab-path]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-tab-path")));
  const openFromTree = async (file) => {
    await page.getByTitle(file, { exact: true }).click();
    await tab("g1", file).waitFor();
  };

  await tab("g0", "docs/source.md").waitFor({ timeout: 10_000 });
  await page.getByRole("link", { name: "Open target" }).click();
  await tab("g0", "docs/target.md").waitFor();
  assert.deepEqual(
    await tabPaths("g0"),
    ["docs/source.md", "docs/target.md"],
    "a Markdown file link must add and activate its target in the source pane",
  );
  assert.equal(await tab("g0", "docs/source.md").count(), 1, "the source document must remain available after following its link");
  assert.equal(await pane("g0").locator('[data-tab-path="docs/target.md"].bg-bg').count(), 1, "the linked target must become active in the source pane");

  await pane("g0").getByRole("button", { name: "오른쪽으로 분할" }).click();
  await page.waitForFunction(() => document.querySelectorAll("[data-group-id]").length === 2);
  await tab("g1", "docs/target.md").waitFor();

  // FileTree expands root-level directories when its asynchronous tree first arrives.
  await page.getByTitle("docs/preview-a.md", { exact: true }).waitFor();
  await openFromTree("docs/preview-a.md");
  await tab("g1", "docs/target.md").getByRole("button", { name: "닫기" }).click();
  await page.waitForFunction(() => document.querySelector('[data-group-id="g1"] [data-tab-path="docs/target.md"]') == null);
  assert.deepEqual(await tabPaths("g1"), ["docs/preview-a.md"], "the second pane must contain only its preview before replacement");

  await openFromTree("docs/preview-b.md");
  assert.equal(await page.locator("[data-group-id]").count(), 2, "replacing a lone preview must preserve the split");
  assert.deepEqual(await tabPaths("g1"), ["docs/preview-b.md"]);

  await openFromTree("docs/target.md");
  assert.equal(await page.locator("[data-group-id]").count(), 2, "opening an existing tab from the tree must preserve the split");
  assert.equal(await tab("g0", "docs/target.md").count(), 1, "the target remains open in its original pane");
  assert.equal(await tab("g1", "docs/target.md").count(), 1, "the tree target must open in the focused pane");
  assert.equal(await pane("g1").locator('[data-tab-path="docs/target.md"].bg-bg').count(), 1, "the focused pane must activate the tree target");

  await page.goto(server.resolvedUrls.local[0] + "index.html?preview=off", {
    waitUntil: "domcontentloaded",
    timeout: 10_000,
  });
  await tab("g0", "docs/source.md").waitFor({ timeout: 10_000 });
  await page.getByRole("link", { name: "Open target" }).click();
  await tab("g0", "docs/target.md").waitFor();
  await pane("g0").getByRole("button", { name: "오른쪽으로 분할" }).click();
  await page.waitForFunction(() => document.querySelectorAll("[data-group-id]").length === 2);
  await page.getByTitle("docs/preview-a.md", { exact: true }).waitFor();
  await openFromTree("docs/preview-a.md");
  assert.equal(await page.locator("[data-group-id]").count(), 2, "tree open with previews disabled must preserve the split");
  assert.deepEqual(await tabPaths("g1"), ["docs/target.md", "docs/preview-a.md"]);
  assert.equal((await tab("g1", "docs/preview-a.md").getAttribute("title"))?.includes("미리보기") ?? false, false, "the disabled preview setting must open a fixed tab");
  assert.deepEqual(errors, [], `browser errors: ${errors.join("\\n")}`);
  console.log("PASS: Markdown links stay in their source pane; per-pane preview replacement and tree focus preserve the split.");
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

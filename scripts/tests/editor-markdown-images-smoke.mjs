import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const run = promisify(execFile);

await mkdir(path.resolve(".praxis/verification"), { recursive: true });
const directory = await mkdtemp(path.resolve(".praxis/verification/editor-images-"));
const fixture = path.join(directory, "ui");
const source = (file) => `/@fs/${path.resolve(file)}`;
const svg = 'data:image/svg+xml;base64,' + Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="96" height="48"><rect width="96" height="48" fill="red"/></svg>').toString("base64");
const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
const markdown = [
  "# Images",
  "![relative](images/chart.svg)",
  "![parent](../assets/pixel.png)",
  "![absolute](/workspace/docs/images/chart.svg)",
  "![uri](file:///workspace/docs/images/chart.svg)",
  "![encoded](images/%ED%95%9C%EA%B8%80%20chart.svg)",
  "![missing](images/missing.png)",
].join("\n\n");
let server;
let browser;
try {
  await mkdir(fixture, { recursive: true });
  const config = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
  // The production image policy must accept the same data URLs as the native webview.
  const imagePolicy = config.app.security.csp.split(";").find((part) => part.trim().startsWith("img-src"));
  assert.ok(imagePolicy);
  await writeFile(path.join(fixture, "index.html"), `<!doctype html><meta http-equiv="Content-Security-Policy" content="${imagePolicy}"><link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>`);
  await writeFile(path.join(fixture, "core.ts"), "export const invoke = async () => null;");
  await writeFile(path.join(fixture, "opener.ts"), "export const openUrl = async () => undefined;");
  await writeFile(path.join(fixture, "main.tsx"), `
import React, { useEffect } from "react";
import { createRoot } from "react-dom/client";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { EditorSplitView } from "${source("src/components/ide/EditorSplitView.tsx")}";
import { useWorkspaceFiles } from "${source("src/components/ide/useWorkspaceFiles.ts")}";

window.__images = { reads: [], windowId: new URLSearchParams(location.search).get("window") ?? "main" };
const workspace = {
  key: "project:images", tree: async () => [], write: async () => 1,
  read: async (path) => {
    window.__images.reads.push(path);
    if (path === "docs/report.md" || path === "docs/zoom.md") return { kind: "text", content: ${JSON.stringify(markdown)}, mtime: 1 };
    const resources = ${JSON.stringify({"docs/images/chart.svg":svg,"assets/pixel.png":png,"docs/images/한글 chart.svg":svg})};
    if (!resources[path]) throw new Error("missing fixture image");
    return { kind: "image", content: resources[path], mtime: 1 };
  },
};
function Harness() {
  const files = useWorkspaceFiles({ task: null, source: workspace, onError: (error) => { throw new Error(error); } });
  useEffect(() => { void files.openFile("docs/report.md"); }, [files.openFile]);
  return <>
    <button onClick={() => void files.openFile("docs/zoom.md")}>줌아웃 파일 열기</button>
    <main style={{ display: "flex", height: "700px", width: "1000px" }}>
      <EditorSplitView taskId={null} host="local" rootPath="/workspace" windowId={new URLSearchParams(location.search).get("window") ?? "main"}
        ownsWindow files={files.openFiles} activeKey={files.activeKey} dark={false} readFile={files.readFile}
        onSelect={files.setActiveKey} onClose={files.closeTab} onOpenFile={files.openFile}
        onChange={files.changeFile} onSave={() => undefined} onReload={() => undefined} onOpenPath={() => undefined} onRevealPath={() => undefined} />
    </main>
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
  const useWebKit = process.env.PRAXIS_SMOKE_BROWSER === "webkit";
  browser = await (useWebKit ? webkit : chromium).launch(useWebKit ? { headless: true } : { headless: true, channel: "chrome" });
  const page = await browser.newPage();
  page.setDefaultTimeout(10_000);
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  for (const windowId of ["main", "editor"]) {
    await page.goto(server.resolvedUrls.local[0] + `index.html?window=${windowId}`);
    const assertImages = async (groups) => {
      await page.waitForFunction((groups) => {
        const images = [...document.querySelectorAll("img")];
        return images.length === 5 * groups && images.every((img) => img.complete && img.naturalWidth > 0);
      }, groups);
      const images = await page.locator("img").evaluateAll((nodes) => nodes.map((img) => ({ alt: img.alt, width: img.naturalWidth, src: img.getAttribute("src") })));
      assert.ok(images.every((img) => img.src.startsWith("data:image/")));
      assert.equal(images.filter((img) => img.width === 96).length, 4 * groups);
      assert.equal(images.filter((img) => img.width === 1).length, groups);
      assert.equal(await page.getByText("[이미지를 불러오지 못했습니다: missing]", { exact: true }).count(), groups);
    };
    await assertImages(1);
    assert.equal(await page.locator("[data-tab-path]").count(), 1, "image reads must not open file tabs");
    await page.getByRole("button", { name: "줌아웃 파일 열기" }).click();
    await page.locator('[data-tab-path="docs/zoom.md"].bg-bg').waitFor();
    await assertImages(1);
    await page.getByRole("button", { name: "오른쪽으로 분할" }).click();
    await assertImages(2);
    const reads = await page.evaluate(() => window.__images.reads);
    assert.ok(reads.includes("docs/images/chart.svg"));
    assert.ok(reads.includes("assets/pixel.png"));
    assert.ok(reads.includes("docs/images/한글 chart.svg"));
    assert.deepEqual(errors, []);
    console.log(`PASS ${useWebKit ? "WebKit" : "Chromium"} ${windowId}: relative, parent, absolute, file URI, encoded images decode under production img-src; zoom and split; missing image fallback`);
  }
  if (process.platform === "darwin") {
    const executable = path.join(directory, "native-images");
    await run("swiftc", ["scripts/tests/editor-markdown-images-native.swift", "-o", executable], { timeout: 60_000 });
    const { stdout } = await run(executable, [server.resolvedUrls.local[0] + "index.html"], { timeout: 45_000 });
    const results = JSON.parse(stdout.trim());
    assert.equal(results.length, 6);
    for (const { images, groups, missing, tabs, stage } of results) {
      assert.equal(images.length, groups * 5);
      assert.ok(images.every((img) => img.src.startsWith("data:image/")));
      assert.equal(images.filter((img) => img.width === 96).length, groups * 4);
      assert.equal(images.filter((img) => img.width === 1).length, groups);
      assert.equal(missing, groups);
      assert.equal(tabs, stage === "initial" ? 1 : stage === "zoom" ? 2 : 3);
    }
    console.log("PASS native WKWebView main/editor: PNG and SVG decode under production img-src, initial/zoom/split, missing image fallback, no image tabs");
  }
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

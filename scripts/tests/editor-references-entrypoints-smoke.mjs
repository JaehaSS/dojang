import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium, webkit } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".praxis/verification/editor-references-entrypoints");
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

const self = { path: "src/Main.java", abs_path: "/workspace/src/Main.java", line: 1, column: 14, external: false };
const target = { path: "src/Other.java", abs_path: "/workspace/src/Other.java", line: 7, column: 3, external: false };
function Harness() {
  const content = "public class Main { void run() {} }";
  const file = { key: fileTabKey("src/Main.java"), path: "src/Main.java", kind: "text", content, baseContent: "const saved = true;", mtime: 0, dirty: true };
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
      onGoto={async (request) => { window.__gotoRequests.push(request); return request.kind === "definition" ? [self] : [target]; }}
      onReferences={(targets) => { window.__refs = (window.__refs ?? []).concat([targets]); }}
      onOpenTarget={async (value) => { window.__opened.push(value); return "opened"; }}
      onNavigateTarget={(value, origin) => window.__navigations.push({ value, origin })}
    />
  </div>;
}
createRoot(document.getElementById("root")).render(<Harness />);
applyTheme("praxis-dark", false);
`,
);


// 사용처 목록의 세 입구 — ⌘B(선언 위), 선언 위 ⌘클릭, 우클릭 메뉴 — 가 모두 앱의 목록으로
// 모이는지 실제 Monaco에서 본다. Monaco 자체 사용처 UI(1건이면 바로 이동, 열지 않은 파일은
// 미리보기 없음)로 새면 onReferences가 불리지 않는다.
let server;
let browser;
try {
  server = await createServer({
    configFile: false,
    root: fixture,
    plugins: [react()],
    resolve: { alias: [{ find: "@tauri-apps/api/core", replacement: path.join(fixture, "tauri-core.ts") }] },
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  page.setDefaultTimeout(5_000);
  await page.goto(server.resolvedUrls.local[0] + "index.html", { waitUntil: "domcontentloaded", timeout: 10_000 });
  await page.locator(".monaco-editor").waitFor({ timeout: 10_000 });

  // Monaco 커서 레이어가 포인터를 가로채므로 좌표로 누른다.
  const word = page.locator(".view-line").first().getByText("Main", { exact: true });
  await word.waitFor();
  const box = await word.boundingBox();
  const at = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  const reset = () => page.evaluate(() => { window.__gotoRequests.length = 0; window.__refs = []; });
  const kinds = () => page.evaluate(() => window.__gotoRequests.map((request) => request.kind));
  const listed = () => page.waitForFunction(() => (window.__refs ?? []).length === 1);

  await page.mouse.click(at.x, at.y);
  await reset();
  await page.keyboard.press("Meta+b");
  await listed();
  assert.deepEqual((await kinds()).slice(-2), ["definition", "references"]);

  await page.keyboard.press("Escape");
  await reset();
  await page.keyboard.down("Meta");
  await page.mouse.click(at.x, at.y);
  await page.keyboard.up("Meta");
  await listed();
  assert.ok((await kinds()).includes("references"));
  assert.equal(await page.locator(".reference-zone-widget").count(), 0);

  await page.keyboard.press("Escape");
  await reset();
  await page.mouse.click(at.x, at.y, { button: "right" });
  const menu = page.locator(".monaco-menu .action-label");
  await menu.first().waitFor();
  const labels = await menu.allInnerTexts();
  assert.ok(labels.includes("사용처 보기"), labels.join(","));
  assert.ok(!labels.includes("Go to References"), labels.join(","));
  const item = menu.filter({ hasText: "사용처 보기" });
  // 메뉴는 연 직후의 mouseup을 무시한다 — 사람처럼 항목 위로 옮긴 뒤 누른다.
  await item.hover();
  await page.waitForTimeout(300);
  await item.click();
  await listed();
  assert.deepEqual(await kinds(), ["references"]);
  assert.equal(await page.locator(".reference-zone-widget").count(), 0);
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

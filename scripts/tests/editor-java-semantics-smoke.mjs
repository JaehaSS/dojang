import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/editor-java-semantics");
const fixture = path.join(directory, "ui-fixture");
const screenshot = path.resolve(".dojang/verification/java-editor-semantics.png");
const source = (file) => `/@fs/${path.resolve(file)}`;

await mkdir(fixture, { recursive: true });
await writeFile(
  path.join(fixture, "index.html"),
  '<div id="root"></div><script type="module" src="./main.tsx"></script>',
);
await writeFile(path.join(fixture, "tauri-core.ts"), "export const invoke = async () => null;");
await writeFile(
  path.join(fixture, "main.tsx"),
  `
import React from "react";
import { createRoot } from "react-dom/client";
import * as monaco from "monaco-editor";
import "${source("src/index.css")}";
import "${source("src/lib/monaco.ts")}";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import { EditorPane } from "${source("src/components/ide/EditorPane.tsx")}";
import { fileTabKey } from "${source("src/lib/tab-key.ts")}";

const alphaSemantic = {
  legend: { token_types: ["class", "method", "variable"], token_modifiers: [] },
  // Alpha/Beta: Monaco LSP delta token encoding.
  data: [0, 13, 5, 0, 0, 1, 7, 3, 1, 0, 1, 8, 5, 2, 0],
};
const betaSemantic = { ...alphaSemantic, data: [0, 13, 4, 0, 0, 1, 7, 3, 1, 0, 1, 8, 5, 2, 0] };
const alphaContent = "public class Alpha {\\n  void run() {\\n    int value = 1;\\n  }\\n}";
const betaContent = "public class Beta {\\n  void run() {\\n    int value = 2;\\n  }\\n}";
const alpha = { key: fileTabKey("src/Alpha.java"), path: "src/Alpha.java", kind: "text", content: alphaContent, baseContent: alphaContent, mtime: 0, dirty: false };
const beta = { key: fileTabKey("src/Beta.java"), path: "src/Beta.java", kind: "text", content: betaContent, baseContent: betaContent, mtime: 0, dirty: false };
let firstBetaLegend = true;
const semanticCalls = {};
const statusCalls = {};
const onLspStatus = async (path) => {
  statusCalls[path] = (statusCalls[path] ?? 0) + 1;
  return {
    available: true,
    phase: path === "src/Beta.java" ? "ready" : statusCalls[path] === 1 ? "analyzing" : "ready",
    server: "jdtls",
    detail: null,
  };
};
function Pane({ file }) {
  return <div style={{ display: "flex", height: "600px", width: "400px" }}>
    <EditorPane
      taskId={1}
      lspScopeKey="local:project:/workspace"
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
      onLspStatus={onLspStatus}
      onSemanticTokens={async (path, text) => {
        window.__semanticRequests.push({ path, text });
        semanticCalls[path] = (semanticCalls[path] ?? 0) + 1;
        // Alpha dispatcher가 먼저 등록된 뒤 Beta handler가 들어오는 상황을 만든다.
        // onDidChange가 없으면 이미 null을 받은 Beta 모델은 편집 전까지 다시 조회되지 않는다.
        if (path === "src/Beta.java" && firstBetaLegend) {
          firstBetaLegend = false;
          const betaModel = monaco.editor.getModels().find((model) => model.uri.toString().endsWith("src/Beta.java"));
          if (betaModel == null) throw new Error("Beta model missing during initial legend request");
          const line = betaModel.getLineCount();
          const column = betaModel.getLineMaxColumn(line);
          betaModel.applyEdits([{
            range: new monaco.Range(line, column, line, column),
            text: "\\n// edited during startup",
          }]);
          window.__editedDuringLegend = true;
          await new Promise((resolve) => setTimeout(resolve, 300));
        }
        // jdtls 초기 분석 중에는 legend만 있고 data가 비는 응답을 낼 수 있다.
        // ready 전환의 dispatcher refresh가 편집 없이 두 번째 실제 token 요청을 만들어야 한다.
        if (semanticCalls[path] === 1) {
          const semantic = path === "src/Beta.java" ? betaSemantic : alphaSemantic;
          return { ...semantic, data: [] };
        }
        return path === "src/Beta.java" ? betaSemantic : alphaSemantic;
      }}
    />
  </div>;
}
function Harness() {
  window.__semanticRequests = window.__semanticRequests ?? [];
  // Alpha를 두 pane에서 열고 Beta를 나란히 연다. dispatcher가 정확한 모델 handler를 골라야 한다.
  return <div style={{ display: "flex", height: "600px", width: "1200px" }}><Pane file={alpha} /><Pane file={beta} /><Pane file={alpha} /></div>;
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
  browser = await chromium.launch({
    headless: true,
    channel: process.env.DOJANG_SMOKE_BROWSER || "chrome",
  });
  const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
  page.setDefaultTimeout(10_000);
  await page.goto(server.resolvedUrls.local[0] + "index.html", { waitUntil: "domcontentloaded" });
  await page.locator(".monaco-editor").first().waitFor();
  await page.waitForFunction(() =>
    window.__semanticRequests?.filter((request) => request.path === "src/Alpha.java").length >= 2 &&
    window.__semanticRequests?.filter((request) => request.path === "src/Beta.java").length >= 2,
  );

  const tokenStyles = await page.locator(".view-line").evaluateAll((lines) => {
    const named = ["Alpha", "Beta", "run", "value"];
    return Object.fromEntries(named.map((name) => {
      const span = [...lines].flatMap((line) => [...line.querySelectorAll("span")])
        .find((candidate) => candidate.textContent === name);
      return [name, span == null ? null : {
        className: span.className,
        color: getComputedStyle(span).color,
      }];
    }));
  });
  const highlightedAlphaCount = await page.locator(".view-line").evaluateAll((lines) =>
    lines.flatMap((line) => [...line.querySelectorAll("span")])
      .filter((span) => span.textContent === "Alpha" && getComputedStyle(span).color === "rgb(78, 201, 176)")
      .length,
  );
  const lineHeight = await page.locator(".view-line").first().evaluate(
    (line) => getComputedStyle(line).lineHeight,
  );
  assert.deepEqual(
    await page.evaluate(() => window.__semanticRequests.some((request) => request.path === "src/Alpha.java" && request.text === "public class Alpha {\n  void run() {\n    int value = 1;\n  }\n}")),
    true,
  );
  assert.equal(await page.evaluate(() => window.__editedDuringLegend), true);
  assert.equal(await page.evaluate(() => window.__semanticRequests.some((request) =>
    request.path === "src/Beta.java" && request.text.endsWith("// edited during startup"),
  )), true);
  const renderedLines = await page.locator(".view-line").evaluateAll((lines) =>
    lines.map((line) => line.innerHTML),
  );
  assert.ok(
    tokenStyles.Alpha && tokenStyles.Beta && tokenStyles.run && tokenStyles.value,
    JSON.stringify({ tokenStyles, renderedLines }),
  );
  assert.notEqual(tokenStyles.Alpha.color, tokenStyles.run.color, JSON.stringify(tokenStyles));
  assert.notEqual(tokenStyles.run.color, tokenStyles.value.color, JSON.stringify(tokenStyles));
  assert.equal(highlightedAlphaCount, 2);
  assert.equal(lineHeight, "21px");
  const settledRequests = await page.evaluate(() => window.__semanticRequests.length);
  await page.waitForTimeout(750);
  assert.equal(await page.evaluate(() => window.__semanticRequests.length), settledRequests);
  await page.screenshot({ path: screenshot });
  console.log(JSON.stringify({
    semanticRequests: settledRequests,
    tokenStyles,
    highlightedAlphaCount,
    lineHeight,
    screenshot,
  }));
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

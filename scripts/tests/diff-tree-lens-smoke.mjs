import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

// 변경 목록 = 트리 열 렌즈, diff 본문 = 코드 열의 diff 탭(ADR 2026-10-08). 실제 컴포넌트를
// 조립하고 네이티브 transport만 가짜로 둔다 — useWorkspaceFiles·DiffSessionProvider·ChangesList·
// EditorSplitView(DiffTab)·WorkspaceSplit·ConversationView·AgentComposer가 그대로 돈다.
const directory = path.resolve(".dojang/verification/diff-tree-lens-smoke");
const fixture = path.join(directory, "ui-fixture");
await mkdir(fixture, { recursive: true });
await writeFile(
  path.join(fixture, "index.html"),
  '<div id="root"></div><script type="module" src="./main.tsx"></script>',
);
await writeFile(
  path.join(fixture, "main.tsx"),
  String.raw`
import React, { useCallback, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { registerTransport } from "/src/lib/transport";
import { DiffSessionProvider } from "/src/components/DiffSessionContext";
import { useSessionPanels } from "/src/components/ide/useSessionPanels";
import { useWorkspaceFiles } from "/src/components/ide/useWorkspaceFiles";
import { WorkspaceSplit } from "/src/components/ide/WorkspaceSplit";
import { CodeColumnTabs } from "/src/components/ide/CodeColumnTabs";
import { ChangesList } from "/src/components/ide/ChangesList";
import { EditorSplitView } from "/src/components/ide/EditorSplitView";
import { ConversationView } from "/src/components/ide/ConversationView";
import { AgentComposer } from "/src/components/ide/AgentComposer";
import "/src/index.css";

const files = [
  { path: "src/alpha.ts", status: "M", patch: "@@ -1 +1 @@\\n-export const alpha = 1;\\n+export const alpha = 2;" },
  { path: "src/beta.ts", status: "A", patch: "@@ -0,0 +1 @@\\n+export const beta = true;" },
];
// DiffHunk(ipc.ts) 모양 그대로 — lines는 {kind, text} 객체다. 문자열로 주면 본문이 빈 줄로 그려진다.
const hunks = [
  { id: "h-alpha", path: "src/alpha.ts", old_range: [1, 1], new_range: [1, 1], protected: false, committed: false, risk: "low",
    lines: [{ kind: "del", text: "export const alpha = 1;" }, { kind: "add", text: "export const alpha = 2;" }] },
  { id: "h-beta", path: "src/beta.ts", old_range: [0, 0], new_range: [1, 1], protected: false, committed: false, risk: "low",
    lines: [{ kind: "add", text: "export const beta = true;" }] },
];

// 조립된 컴포넌트가 닿는 경계 전부. Tauri 프로세스도 저장소도 없다 — 모르는 호출은 빈 목록이다.
const base = {
  hostId: "smoke",
  kind: "remote",
  taskDiff: async () => ({ files, baseline: { kind: "pinned" } }),
  diffHunks: async () => hunks,
  annotationsList: async () => [],
  skillsList: async () => [],
  knowledgeSearch: async () => [],
  fileTree: async () => [],
};
registerTransport(new Proxy(base, {
  get: (target, key) => (key in target ? target[key as keyof typeof target] : async () => []),
}) as any);

const baseItems = Array.from({ length: 90 }, (_, index) => ({
  role: "text" as const,
  text: "대화 행 " + index + " — diff를 보는 동안에도 유지되어야 하는 충분히 긴 대화 내용입니다.",
}));

function SessionContents({ stream }: { stream: number }) {
  const items = useMemo(
    () => [
      ...baseItems,
      ...Array.from({ length: stream }, (_, index) => ({ role: "text" as const, text: "스트리밍 업데이트 " + index })),
    ],
    [stream],
  );
  return (
    <div className="flex min-h-0 flex-1 flex-col" data-chat-retained>
      <ConversationView conversationId={42} items={items} busy hidden={false} />
    </div>
  );
}

function Harness() {
  const panels = useSessionPanels("smoke:42");
  const [stream, setStream] = useState(0);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const openCode = useCallback(() => {
    panels.setCodeTab("file");
    panels.setCodeOpen(true);
  }, [panels]);
  const workspace = useWorkspaceFiles({
    task: { host: "smoke", id: 42 },
    onError: setError,
    onFileOpened: openCode,
  });
  const close = useCallback(() => panels.setCodeOpen(false), [panels]);
  const activate = useCallback((tab: "activity" | "file" | "preview") => {
    panels.setCodeTab(tab);
    panels.setCodeOpen(true);
  }, [panels]);
  const changesOn = panels.showTree && panels.treeLens === "changes";
  const code = (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col" data-code-column>
      <CodeColumnTabs
        active={panels.codeTab}
        onActivate={activate}
        previewAvailable
        editorPoppedOut={false}
        onClose={close}
        activity={<div>작업정보</div>}
        file={
          <EditorSplitView
            host="smoke"
            windowId="smoke"
            files={workspace.openFiles}
            activeKey={workspace.activeKey}
            treeOpen={workspace.treeOpen}
            onTreeOpenHandled={workspace.consumeTreeOpen}
            dark={false}
            onSelect={workspace.setActiveKey}
            onClose={workspace.closeTab}
            onChange={workspace.changeFile}
            onSave={workspace.saveFile}
            onReload={workspace.reloadFile}
            onOpenPath={(p: string) => void workspace.openFile(p)}
            onRevealPath={() => undefined}
            shortcutsActive={panels.codeOpen && panels.codeTab === "file"}
            taskId={42}
          />
        }
        preview={<div>프리뷰</div>}
      />
    </div>
  );
  return (
    <DiffSessionProvider task={{ host: "smoke", id: 42 }} openDiff={workspace.openDiff}>
      <main className="flex h-screen min-w-0 flex-col overflow-hidden bg-bg text-text" data-smoke-shell>
        <header className="flex shrink-0 items-center gap-2 border-b border-border p-2">
          <button
            aria-label={changesOn ? "변경 닫기" : "변경 보기"}
            aria-pressed={changesOn}
            onClick={() => (changesOn ? panels.setShowTree(false) : panels.openChangesLens())}
          >
            변경
          </button>
          <button aria-label="스트리밍 업데이트" onClick={() => setStream((value) => value + 1)}>stream</button>
          {error && <span data-smoke-error>{error}</span>}
        </header>
        <div className="flex min-h-0 flex-1">
          {panels.showTree && (
            <aside className="flex w-52 shrink-0 flex-col border-r border-border bg-surface" data-tree-aside>
              <div className="flex h-8 shrink-0 items-center gap-1 border-b border-border px-3" role="tablist" aria-label="트리 열 렌즈">
                {([["files", "파일"], ["changes", "변경"]] as const).map(([lens, label]) => (
                  <button key={lens} role="tab" aria-selected={panels.treeLens === lens} onClick={() => panels.setTreeLens(lens)}>
                    {label}
                  </button>
                ))}
                <button className="ml-auto" aria-label="파일 트리 닫기" onClick={() => panels.setShowTree(false)}>✕</button>
              </div>
              {panels.treeLens === "changes" ? <ChangesList /> : <div data-file-tree>파일 트리</div>}
            </aside>
          )}
          <WorkspaceSplit
            session={
              <div className="flex min-h-0 flex-1 flex-col" data-session-column>
                <SessionContents stream={stream} />
                <footer data-composer>
                  <AgentComposer
                    value={draft}
                    onChange={setDraft}
                    onSend={() => {}}
                    onInterrupt={() => {}}
                    onHistory={() => {}}
                    files={files.map((file) => file.path)}
                    repo="/mock/repo"
                    taskId={42}
                    host="smoke"
                    active
                  />
                </footer>
              </div>
            }
            code={code}
            codeOpen={panels.codeOpen}
            onCloseCode={close}
          />
        </div>
      </main>
    </DiffSessionProvider>
  );
}

createRoot(document.getElementById("root")!).render(<Harness />);
`,
);

let server;
let browser;
let page;
const outcomes = [];
let consoleLines = [];
try {
  server = await createServer({
    configFile: false,
    plugins: [react()],
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" });
  page = await browser.newPage({ viewport: { width: 1280, height: 760 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  consoleLines = [];
  page.on("console", (message) => consoleLines.push(message.type() + ": " + message.text()));
  const url = server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + "/index.html";
  const aside = page.locator("[data-tree-aside]");
  const codeColumn = page.locator("[data-code-column]");
  const session = page.locator("[data-session-column]");
  const diffTab = (file) => page.locator(`[data-diff-tab="${file}"]`);
  const chat = page.locator("[data-chat-retained]");
  const composer = page.locator("[data-composer] textarea");
  const alpha = aside.getByRole("button", { name: /alpha\.ts/ });
  const beta = aside.getByRole("button", { name: /beta\.ts/ });

  await page.goto(url);
  await composer.fill("보존할 초안");
  await page.getByRole("button", { name: "변경 보기" }).click();
  await alpha.waitFor();
  assert.equal(await aside.getByRole("tab", { name: "변경" }).getAttribute("aria-selected"), "true");
  assert.equal(await codeColumn.count(), 0, "변경 보기는 코드 열을 열지 않는다");
  assert.equal(await page.locator("[data-smoke-error]").count(), 0);
  outcomes.push("변경 보기 opens the tree-column changes lens without opening the code column");

  // 코드 열이 열리면 세션 열이 좁아져 줄바꿈이 달라진다. ConversationView는 폭이 바뀌면 보던 항목을
  // 같은 자리에 다시 놓으므로(WKWebView에는 스크롤 앵커링이 없다) 비교 대상은 scrollTop 숫자가 아니라
  // "맨 위에 보이던 항목"이다. 따라가기(follow)가 켜져 바닥으로 끌려가지 않았는지도 함께 본다.
  const chatScroll = chat.locator(".overflow-y-auto").first();
  const topVisibleItem = () =>
    chatScroll.evaluate((node) => {
      const top = node.scrollTop;
      const hit = [...node.children].find((child) => child.offsetTop + child.offsetHeight > top + 1);
      return {
        text: hit?.textContent ?? null,
        atBottom: node.scrollTop >= node.scrollHeight - node.clientHeight - 1,
      };
    });
  await chatScroll.evaluate((node) => {
    node.scrollTop = 120;
    node.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  const beforeOpen = await topVisibleItem();
  assert.equal(beforeOpen.atBottom, false);
  await alpha.click();
  await diffTab("src/alpha.ts").waitFor();
  assert.equal(await codeColumn.isVisible(), true);
  assert.equal(await alpha.isVisible(), true, "the changes lens stays while the diff tab is open");
  assert.equal(await session.isVisible(), true, "the session column is not covered by the diff");
  await page.getByRole("button", { name: "스트리밍 업데이트" }).click();
  await page.getByRole("button", { name: "스트리밍 업데이트" }).click();
  await page.waitForTimeout(30);
  assert.equal(await chat.count(), 1);
  const afterOpen = await topVisibleItem();
  assert.equal(afterOpen.text, beforeOpen.text, "the reader's top item moved when the code column opened");
  assert.equal(afterOpen.atBottom, false, "streaming dragged the reader to the bottom while a diff tab is open");
  assert.equal(await composer.inputValue(), "보존할 초안");
  const composerFocusable = await composer.evaluate((node) => {
    node.focus();
    return document.activeElement === node;
  });
  assert.equal(composerFocusable, true, "the composer stays live while a diff tab is open");
  outcomes.push("A row opens diff:src/alpha.ts in the code column; chat anchor, draft, and composer focus survive");

  // 구문 강조 — monaco.ts가 주입한 토크나이저가 실제 Chrome에서 돈다(설계 DR-12).
  await page.waitForFunction(
    (file) => [...document.querySelectorAll(`[data-diff-tab="${file}"] span[style*="color"]`)]
      .some((node) => /export|const/.test(node.textContent ?? "")),
    "src/alpha.ts",
    { timeout: 8000 },
  ).catch(() => {
    throw new Error("diff tab shows no syntax-colored keyword span within 8s");
  });
  outcomes.push("diff body gets Monaco-token syntax colors for TypeScript keywords");

  // 파일 단축키 — 활성 diff 탭이 받는다. 포커스가 입력창에 있으면 무시하므로 본문을 먼저 누른다.
  await diffTab("src/alpha.ts").click();
  await page.keyboard.press("]");
  await diffTab("src/beta.ts").waitFor();
  assert.equal(await beta.isVisible(), true);
  assert.equal(await page.locator("[data-tab-key='diff:src/beta.ts']").count(), 1, "beta opened as a diff tab");
  outcomes.push("] advances A to B as another diff tab while the list stays");

  await aside.getByRole("tab", { name: "파일" }).click();
  assert.equal(await alpha.count(), 0, "files lens hides the changes list");
  assert.equal(await page.locator("[data-file-tree]").count(), 1);
  assert.equal(await diffTab("src/beta.ts").isVisible(), true, "the diff tab outlives the lens switch");
  await aside.getByRole("tab", { name: "변경" }).click();
  await alpha.waitFor();
  outcomes.push("lens toggle swaps file tree and changes list without touching the diff tab");

  for (const width of [1280, 1440, 1680]) {
    await page.setViewportSize({ width, height: 760 });
    await page.waitForTimeout(30);
    const [asideBox, sessionBox, codeBox, shell] = await Promise.all([
      aside.boundingBox(),
      session.boundingBox(),
      codeColumn.boundingBox(),
      page.locator("[data-smoke-shell]").evaluate((node) => ({ scrollWidth: node.scrollWidth, clientWidth: node.clientWidth })),
    ]);
    assert(asideBox && sessionBox && codeBox, "missing layout boxes at " + width + "px");
    assert(sessionBox.x >= asideBox.x + asideBox.width - 0.5, "session is not right of the tree at " + width + "px");
    assert(codeBox.x >= sessionBox.x + sessionBox.width - 0.5, "code column is not right of session at " + width + "px");
    assert(codeBox.width >= 479.5, "code column narrower than 480px at " + width + "px: " + codeBox.width);
    assert(shell.scrollWidth <= shell.clientWidth, "workspace overflows at " + width + "px");
    outcomes.push(width + "px keeps tree | session | code in order without horizontal overflow");
  }

  await page.setViewportSize({ width: 1280, height: 760 });
  await page.waitForTimeout(30);
  await page.screenshot({ path: path.join(directory, "diff-tree-lens.png") });
  await page.getByRole("button", { name: "코드 열 닫기" }).click();
  assert.equal(await codeColumn.count(), 0);
  assert.equal(await composer.inputValue(), "보존할 초안");
  assert.equal(await alpha.isVisible(), true, "closing the code column leaves the lens open");
  await page.getByRole("button", { name: "변경 닫기" }).click();
  assert.equal(await aside.count(), 0);
  await page.getByRole("button", { name: "변경 보기" }).click();
  assert.equal(await aside.getByRole("tab", { name: "변경" }).getAttribute("aria-selected"), "true");
  outcomes.push("closing the code column keeps the draft; the lens remembers 변경 across close and reopen");

  assert.equal(await page.locator("[data-smoke-error]").count(), 0);
  assert.deepEqual(errors, []);
  console.log("PASS diff tree-lens smoke (" + outcomes.length + " checks):\n- " + outcomes.join("\n- ") + "\nScreenshot: " + path.join(directory, "diff-tree-lens.png"));
} catch (error) {
  if (page) {
    await page.screenshot({ path: path.join(directory, "diff-tree-lens-failure.png") }).catch(() => {});
    const dump = await page
      .evaluate(() => document.querySelector("[data-diff-tab]")?.outerHTML ?? "(no diff tab)")
      .catch(() => "(page gone)");
    await writeFile(path.join(directory, "diff-tree-lens-failure.html"), dump).catch(() => {});
    await writeFile(path.join(directory, "diff-tree-lens-console.log"), consoleLines.join("\n")).catch(() => {});
  }
  throw error;
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}

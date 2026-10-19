import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";
const dir = path.resolve(".praxis/verification/context-breakdown"), fixture = path.join(dir, "fixture");
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, "index.html"), '<link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, "main.tsx"), String.raw `
import React, { useState, useCallback } from "react";
import { createRoot } from "react-dom/client";
import { ContextChannel } from "/src/components/ide/ContextChannel";
import { ContextGauge } from "/src/components/ide/ContextGauge";
import { applyTheme } from "/src/lib/themes";
import "/src/index.css";

const metrics = {
  version: 1, user_bytes: 111, sent_user_bytes: 222, instruction_bytes: 333,
  skill_added_bytes: 444, goal_added_bytes: 555, capsule_added_bytes: 666,
  vault_added_bytes: 777,
};
const seed = [
  { kind: "user", text: "예시 사용자 요청", prompt_metrics: metrics },
  { kind: "text", text: "예시 응답" },
  { kind: "tool_use", name: "Read", tool_id: "read" },
  { kind: "tool_result", tool_use_id: "read", result_chars: 12000 },
  { kind: "tool_use", name: "Bash", tool_id: "pending" },
  { kind: "tool_use", name: "mcp__sample__lookup", tool_id: "mcp" },
  { kind: "tool_result", tool_use_id: "mcp", result_chars: 321 },
  { kind: "tool_use", name: "functions.exec", tool_id: "other" },
  { kind: "tool_result", tool_use_id: "other", result_chars: 456 },
  { kind: "subagent_update", vendor: "codex", child_thread_id: "child",
    title: "예시 하위 작업", model: "child-model-v1", state: "done" },
  { kind: "tool_result", parent_id: "child", tool_use_id: "child-result", result_chars: 99999 },
  { kind: "tool_result", speaker: "right", tool_use_id: "peer", result_chars: 88888 },
];
const task = {
  id: 701, host: "local", repo: "/example", branch: "main", base: "main",
  worktree_path: "/example/701", instruction: "예시", state: "Working",
  created_at: 1, updated_at: 2, agent: "codex", mode: "conversation",
};
const codexObservation = {
  contextTokens: 129200, contextWindow: 258400,
  observedAt: 1725000000, source: "codex_session",
};
const claudeObservation = {
  contextTokens: 300000, contextWindow: null,
  observedAt: 1725000100, source: "claude_message",
};

function Harness() {
  const [agent, setAgent] = useState("codex");
  const [events, setEvents] = useState(seed);
  const [observation, setObservation] = useState(codexObservation);
  const [busy, setBusy] = useState(false);
  const [active, setActive] = useState(true);
  const [focus, setFocus] = useState(0);
  const [count, setCount] = useState(0);
  const [fail, setFail] = useState(false);
  const requestFocus = () => setFocus(value => value + 1);
  const refresh = useCallback(async () => {
    setCount(value => value + 1);
    if (fail) throw new Error("sample refresh failure");
  }, [fail]);
  const context = {
    agent, model: agent === "claude" ? "opus[1m]" : "codex-observed",
    observation, events: events.map(event => ({ ...event, parent_id: event.parent_id ?? null })),
    busy, onRefresh: refresh, onOpenDetails: requestFocus,
    focusRequest: focus, onFocusHandled: () => {},
  };
  window.__contextSmoke = {
    live: () => setEvents(current => [...current,
      { kind: "tool_use", name: "Grep", tool_id: "live" },
      { kind: "tool_result", tool_use_id: "live", result_chars: 73 },
    ]),
    session: () => setEvents([
      { kind: "user", text: "새 세션", prompt_metrics: metrics },
      { kind: "tool_use", name: "Write", tool_id: "new" },
      { kind: "tool_result", tool_use_id: "new", result_chars: 31 },
    ]),
    clear: () => setEvents([
      { kind: "context_cleared" },
      { kind: "user", text: "비운 뒤", prompt_metrics: { ...metrics, user_bytes: 9 } },
      { kind: "tool_use", name: "Grep", tool_id: "after" },
      { kind: "tool_result", tool_use_id: "after", result_chars: 17 },
    ]),
    claude: () => { setAgent("claude"); setObservation(claudeObservation); },
    missing: () => setObservation(null),
    active: setActive, busy: setBusy, focus: requestFocus,
    fail: setFail, count: () => count,
    theme: id => applyTheme(id, false),
  };
  return (
    <main className="min-h-screen bg-bg p-4 text-text">
      <p>예시 데이터</p>
      <div className="grid grid-cols-2 gap-4">
        <div data-mode="rail">
          <ContextChannel context={context} density="rail" active={active} />
        </div>
        <div data-mode="panel" className="max-h-[640px] overflow-y-auto">
          <ContextChannel context={context} density="panel" active={active} />
          <div className="h-[700px]">작업 정보와 아래 카드</div>
        </div>
      </div>
      <ContextGauge task={task} observation={observation} model={context.model}
        busy={busy} onOpenDetails={requestFocus} />
    </main>
  );
}
createRoot(document.getElementById("root")).render(<Harness />);
`);
let server, browser;
const errors = [], passed = [];
try {
    server = await createServer({ configFile: false, plugins: [react()], server: { host: "127.0.0.1", port: 0 }, logLevel: "error" });
    await server.listen();
    browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" });
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    page.setDefaultTimeout(5000);
    page.on("pageerror", e => errors.push(e.message));
    page.on("console", m => { if (m.type() === "error")
        errors.push(m.text()); });
    await page.goto(server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + "/index.html");
    const rail = page.locator('[data-mode="rail"]'), panel = page.locator('[data-mode="panel"]');
    await rail.getByRole("region", { name: "컨텍스트 요약" }).waitFor();
    assert.equal(await page.getByRole("dialog").count(), 0);
    if (await panel.getByRole("button", { name: "컨텍스트 상세 펼치기" }).count())
        await panel.getByRole("button", { name: "컨텍스트 상세 펼치기" }).click();
    const detail = panel.getByRole("region", { name: "컨텍스트 상세" });
    await detail.getByText("333 B", { exact: true }).waitFor();
    await detail.getByText("12,000자", { exact: false }).first().waitFor();
    assert.equal(await detail.getByText("99,999", { exact: false }).count(), 0);
    assert.equal(await detail.getByText("88,888", { exact: false }).count(), 0);
    for (const x of ["파일", "명령", "MCP", "기타"])
        await detail.locator("summary").filter({ hasText: x }).click();
    await detail.getByText("functions.exec", { exact: true }).waitFor();
    await detail.getByText("예시 하위 작업", { exact: true }).waitFor();
    passed.push("exact units, groups, and isolated child records");
    await page.evaluate(() => window.__contextSmoke.live());
    await detail.locator("summary").filter({ hasText: "검색" }).click();
    await detail.getByText("Grep", { exact: true }).waitFor();
    await page.evaluate(() => window.__contextSmoke.session());
    await detail.locator("summary").filter({ hasText: "파일" }).click();
    assert.ok(await detail.getByText("Write", { exact: true }).count() > 0);
    assert.equal(await detail.getByText("Read", { exact: true }).count(), 0);
    await page.evaluate(() => window.__contextSmoke.clear());
    await detail.locator("summary").filter({ hasText: "검색" }).click();
    assert.equal(await detail.getByText("111 B", { exact: true }).count(), 0);
    await detail.getByText("9 B", { exact: true }).waitFor();
    passed.push("live records, session replacement, and reset");
    await page.evaluate(() => window.__contextSmoke.active(false));
    const before = await page.evaluate(() => window.__contextSmoke.count());
    await page.waitForTimeout(80);
    assert.equal(await page.evaluate(() => window.__contextSmoke.count()), before);
    await page.evaluate(() => { window.__contextSmoke.active(true); window.__contextSmoke.busy(true); });
    await page.waitForTimeout(80);
    assert.equal(await page.evaluate(() => window.__contextSmoke.count()), before);
    await page.evaluate(() => window.__contextSmoke.busy(false));
    await page.waitForFunction(x => window.__contextSmoke.count() > x, before);
    await page.evaluate(() => window.__contextSmoke.fail(true));
    await panel.getByRole("button", { name: "컨텍스트 기록 새로고침" }).click();
    await panel.getByRole("alert").waitFor();
    await page.evaluate(() => { window.__contextSmoke.fail(false); window.__contextSmoke.focus(); });
    await page.waitForFunction(() => document.activeElement?.hasAttribute("data-context-summary"));
    await panel.getByRole("button", { name: "컨텍스트 상세 접기" }).waitFor();
    passed.push("active idle refresh, error, and focus");
    await page.getByRole("button", { name: /컨텍스트 구성 보기/ }).click();
    await page.waitForFunction(() => document.activeElement?.hasAttribute("data-context-summary"));
    await page.evaluate(() => window.__contextSmoke.claude());
    await panel.getByText("300,000", { exact: true }).waitFor();
    await panel.getByText("1,000,000 토큰", { exact: false }).waitFor();
    await page.evaluate(() => { window.__contextSmoke.missing(); window.__contextSmoke.theme("praxis-dark"); });
    await page.screenshot({ path: path.join(dir, "channel-dark.png"), fullPage: true });
    await page.evaluate(() => window.__contextSmoke.theme("praxis-light"));
    await page.screenshot({ path: path.join(dir, "channel-light.png"), fullPage: true });
    await page.setViewportSize({ width: 375, height: 667 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
    await page.screenshot({ path: path.join(dir, "channel-narrow.png"), fullPage: true });
    assert.deepEqual(errors, []);
    const result = { passed, browserErrors: errors, screenshots: ["channel-dark.png", "channel-light.png", "channel-narrow.png"].map(x => path.join(dir, x)), limitation: "Real channel with labelled samples; native IPC is covered by context-workspace-smoke." };
    await writeFile(path.join(dir, "summary.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result, null, 2));
}
finally {
    await browser?.close();
    await server?.close();
    await rm(fixture, { recursive: true, force: true });
}

import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/tap-tap-navigation-smoke");
const fixture = path.join(directory, "ui-fixture");
const darkScreenshot = "/tmp/praxis-session-navigator-dark.png";
const lightScreenshot = "/tmp/praxis-session-navigator-light.png";
await mkdir(fixture, { recursive: true });
await writeFile(path.join(fixture, "index.html"), '<div id="root"></div><script type="module" src="./main.tsx"></script>');
await writeFile(path.join(fixture, "main.tsx"), `
import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { SessionNavigator } from "/src/components/SessionNavigator";
import { onKeyDown, onKeyUp } from "/src/lib/shift-double-tap";
import { applyTheme } from "/src/lib/themes";
import "/src/index.css";
applyTheme("praxis-dark", false);
window.setSmokeTheme = id => applyTheme(id, false);
const alpha = "/workspace/alpha", beta = "/workspace/beta";
const task = (host, id) => ({ id, host, repo:alpha, branch:"main", base:"main", worktree_path:alpha, instruction:"session-" + host + "-" + id, state:"Running", created_at:id, updated_at:id, mode:"conversation" });
const groups = { version:1, groups:[{ id:"team", name:"Team", collapsed:true }], assignment:{ [alpha]:"team" } };
function Harness() {
  const [open, setOpen] = useState(false), [opened, setOpened] = useState("");
  useEffect(() => {
    let tap = { lastUp:0 };
    const down = event => { tap = onKeyDown(tap, event); };
    const up = event => { const result = onKeyUp(tap, event, performance.now()); tap = result.next; if (result.fire) setOpen(true); };
    window.addEventListener("keydown", down); window.addEventListener("keyup", up);
    return () => { window.removeEventListener("keydown", down); window.removeEventListener("keyup", up); };
  }, []);
  return <main className="bg-bg text-text p-4"><button id="origin">origin</button><output data-opened>{opened}</output><SessionNavigator open={open} tasks={[task("local", 7), task("remote", 7)]} projects={[alpha, beta]} groups={groups} onClose={() => setOpen(false)} onOpenTask={target => { setOpened(target.host + ":" + target.id); setOpen(false); }} /></main>;
}
createRoot(document.getElementById("root")).render(<Harness />);
`);

let server;
let browser;
try {
  server = await createServer({ configFile: false, plugins: [react()], server: { host: "127.0.0.1", port: 0 }, logLevel: "error" });
  await server.listen();
  browser = await chromium.launch({ headless: true, channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome" });
  const page = await browser.newPage({ viewport: { width: 900, height: 650 } });
  await page.goto(server.resolvedUrls.local[0] + path.relative(process.cwd(), fixture) + "/index.html");
  await page.locator("#origin").focus();
  await page.keyboard.press("Shift");
  await page.keyboard.press("Shift");
  const dialog = page.getByRole("dialog", { name: "세션 탐색" });
  await dialog.waitFor();
  await page.getByLabel("세션 탐색 검색").fill("remote");
  await assert.doesNotReject(() => page.getByText("Team", { exact: true }).waitFor());
  await assert.doesNotReject(() => page.getByText("alpha", { exact: true }).waitFor());
  await assert.doesNotReject(() => page.getByText("session-remote-7", { exact: true }).waitFor());
  await page.getByLabel("세션 탐색 검색").fill("");
  const team = page.getByRole("treeitem", { name: "Team" });
  await team.focus();
  await team.press("ArrowRight");
  await team.press("ArrowRight");
  await page.getByRole("treeitem", { name: /alpha/ }).press("ArrowRight");
  await page.getByRole("treeitem", { name: /session-local-7/ }).press("ArrowDown");
  await page.getByRole("treeitem", { name: /session-remote-7/ }).press("Enter");
  assert.equal(await page.locator("[data-opened]").textContent(), "remote:7");
  await page.locator("#origin").focus();
  await page.keyboard.press("Shift");
  await page.keyboard.press("Shift");
  await dialog.waitFor();
  await page.screenshot({ path: darkScreenshot });
  await page.evaluate(() => window.setSmokeTheme("praxis-light"));
  await page.screenshot({ path: lightScreenshot });
  await page.getByLabel("세션 탐색 검색").press("Escape");
  await dialog.waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => document.activeElement?.id), "origin");
  console.log(`PASS tap-tap navigation: collapsed search, host-qualified open, keyboard tree, escape focus, screenshots ${darkScreenshot}, ${lightScreenshot}`);
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

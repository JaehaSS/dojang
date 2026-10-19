import assert from "node:assert/strict";
import { createServer as createHttpServer } from "node:http";
import { writeFile, unlink } from "node:fs/promises";
import { resolve } from "node:path";
import { randomUUID } from "node:crypto";
import { createServer } from "vite";
import { chromium } from "playwright";

// Exercise the real dev-only entry and HMR without Tauri IPC or a file-write endpoint.
const slug = `dojang-smoke-${randomUUID()}`;
const fixture = resolve(`src/design/screens/${slug}.screen.tsx`);
let fixtureCreated = false;
let occupied;
let server;
let browser;
const errors = [];
const evidence = {};
try {
  occupied = createHttpServer((_request, response) => response.end("1420 occupied"));
  try {
    await new Promise((resolve, reject) => { occupied.once("error", reject); occupied.listen(1420, "127.0.0.1", resolve); });
    evidence.port1420 = "occupied by test";
  } catch (error) {
    if (error.code !== "EADDRINUSE") throw error;
    occupied = null;
    evidence.port1420 = "already occupied";
  }
  server = await createServer({ mode: "design", server: { port: 0, host: "127.0.0.1", strictPort: true }, clearScreen: false });
  await server.listen();
  const port = server.httpServer.address().port;
  assert.notEqual(port, 1420);
  evidence.canvasPort = port;
  browser = await chromium.launch({ headless: true, channel: "chrome" });
  const page = await browser.newPage({ viewport: { width: 1280, height: 820 } });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${port}/design.html`);
  await page.getByRole("button", { name: "주간 통계 카드", exact: true }).waitFor();
  const frame = page.frameLocator("iframe");
  await frame.getByRole("heading", { name: "이번 주 작업" }).waitFor();
  evidence.defaultViewport = await page.locator("iframe").evaluate((element) => [element.contentWindow.innerWidth, element.contentWindow.innerHeight]);
  assert.deepEqual(evidence.defaultViewport, [960, 640]);
  await page.getByRole("button", { name: "빈 상태", exact: true }).click();
  await frame.getByText("아직 기록된 작업이 없습니다.").waitFor();
  await page.getByRole("button", { name: "오류", exact: true }).click();
  await frame.getByRole("alert").filter({ hasText: "통계를 불러오지 못했습니다." }).waitFor();
  await page.getByRole("button", { name: "정상", exact: true }).click();
  await page.getByLabel("뷰포트", { exact: true }).selectOption("390x844");
  await page.waitForFunction(() => document.querySelector("iframe").contentWindow.innerWidth === 390);
  evidence.alternateViewport = await page.locator("iframe").evaluate((element) => [element.contentWindow.innerWidth, element.contentWindow.innerHeight]);
  assert.deepEqual(evidence.alternateViewport, [390, 844]);
  await page.getByLabel("배율", { exact: true }).selectOption("50");
  assert.equal(await page.locator("iframe").evaluate((element) => element.contentWindow.innerWidth), 390);
  evidence.statesAndZoom = true;
  const metadata = 'export const meta = {title:"자동 수집 검증",viewport:{width:640,height:480},states:{정상:{label:"실제 props"}}};';
  await writeFile(fixture, `${metadata}\nexport default function Screen({label}: {label: string}) {return <p>{label}</p>;}`, { flag: "wx" });
  fixtureCreated = true;
  await page.getByRole("button", { name: "자동 수집 검증", exact: true }).waitFor();
  await page.getByRole("button", { name: "자동 수집 검증", exact: true }).click();
  await frame.getByText("실제 props", { exact: true }).waitFor();
  assert.deepEqual(await page.locator("iframe").evaluate((element) => [element.contentWindow.innerWidth, element.contentWindow.innerHeight]), [640, 480]);
  evidence.hotRegistration = true;
  await writeFile(fixture, 'export const meta = {title:"깨진 화면",viewport:{width:0,height:480},states:{정상:{}}};\nexport default function Screen(){throw Error("must not render");}');
  await page.getByRole("alert").filter({ hasText: "양의 정수" }).first().waitFor();
  await page.getByRole("button", { name: slug, exact: true }).click();
  assert.equal(await page.locator("iframe").count(), 0);
  assert.ok(!errors.some((error) => error.includes("must not render")));
  evidence.invalidMetaSkipped = true;
  await unlink(fixture);
  await page.getByRole("button", { name: "주간 통계 카드", exact: true }).waitFor();
  await page.waitForFunction((removed) => ![...document.querySelectorAll("button")].some((button) => button.textContent === removed), slug);
  evidence.hotRemoval = true;
  await page.screenshot({ path: "/tmp/dojang-design-canvas-smoke.png" });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify(evidence, null, 2));
} finally {
  if (fixtureCreated) await unlink(fixture).catch((error) => { if (error.code !== "ENOENT") throw error; });
  await browser?.close();
  await server?.close();
  if (occupied?.listening) { occupied.closeAllConnections(); await new Promise((resolve) => occupied.close(resolve)); }
}

import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { writeFile, unlink } from "node:fs/promises";
import { resolve } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright";

const slug = `dojang-write-${randomUUID()}`;
const screenFile = resolve(`src/design/screens/${slug}.screen.tsx`);
const componentFile = resolve(`src/design/screens/${slug}.tsx`);
const original = `import Badge from "./${slug}";
export const meta = {title:"기록 갱신 검증",viewport:{width:640,height:640},states:{정상:{}}};
export default function Screen() {return <main>
 <div data-testid="static" className="p-4">정적 요소</div>
 <Badge testId="one" className="p-2" />
 <Badge testId="two" className="p-3" />
 {[1,2].map((id)=><div key={id} data-testid="duplicate" className="p-4">반복 요소</div>)}
</main>;}
`;
let server, browser; const created = []; const errors = [];
try {
  await writeFile(componentFile, 'export default function Badge({testId,className}:{testId:string;className?:string}) {return <div data-testid={testId} className={className}>사용처</div>}', { flag: "wx" }); created.push(componentFile);
  await writeFile(screenFile, original, { flag: "wx" }); created.push(screenFile);
  server = await createServer({ mode: "design", server: { host: "127.0.0.1", port: 0, strictPort: true }, clearScreen: false }); await server.listen();
  browser = await chromium.launch({ headless: true, channel: "chrome" });
  const page = await browser.newPage({ viewport: { width: 1400, height: 950 } }); page.on("pageerror", (error) => errors.push(error.message));
  let rootNavigations = 0; page.on("framenavigated", (frame) => { if (frame === page.mainFrame()) rootNavigations++; });
  await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/design.html`);
  await page.getByRole("button", { name: "기록 갱신 검증", exact: true }).click();
  const frame = page.frameLocator("iframe"); const inspector = page.locator('[aria-label="인스펙터"]');
  const pick = async (name) => { await page.getByRole("button", { name: "요소 선택", exact: true }).click(); await frame.getByTestId(name).first().click({ position: { x: 3, y: 3 } }); await inspector.locator('[aria-label="소스 위치"]').waitFor(); };
  await pick("one");
  const first = await page.evaluate(() => JSON.parse(sessionStorage.getItem("dojang-design-canvas-resume")));
  await pick("two");
  const second = await page.evaluate(() => JSON.parse(sessionStorage.getItem("dojang-design-canvas-resume")));
  assert.ok(first.instanceSource && second.instanceSource);
  assert.equal(first.source, second.source); assert.notEqual(first.instanceSource, second.instanceSource);
  await pick("static"); const source = await inspector.locator('[aria-label="소스 위치"]').textContent();
  const initialNavigations = rootNavigations;
  // The test runner simulates a successful Rust write; the dev server has no write endpoint.
  await writeFile(screenFile, original.replace('data-testid="static" className="p-4"', 'data-testid="static" className="p-8"'));
  await page.waitForFunction(() => document.querySelector('[aria-label="padding 박스"]')?.textContent?.includes("32px"), { timeout: 15000 });
  assert.equal(await inspector.locator('[aria-label="소스 위치"]').textContent(), source);
  assert.equal(rootNavigations, initialNavigations, "class-only write should preserve inspector focus without root reload");
  await writeFile(screenFile, original.replace('data-testid="static" className="p-4"', 'data-testid="static" className="p-8"').replace("기록 갱신 검증", "갱신 메타데이터"));
  await page.getByRole("button", { name: "갱신 메타데이터", exact: true }).waitFor();
  await page.waitForFunction(() => document.querySelector('[aria-label="padding 박스"]')?.textContent?.includes("32px"));
  assert.equal(await inspector.locator('[aria-label="소스 위치"]').textContent(), source);
  await pick("duplicate");
  await writeFile(screenFile, original.replace("정적 요소", "갱신 요소"));
  await page.getByRole("alert").filter({ hasText: "같은 소스 위치" }).waitFor({ timeout: 15000 });
  assert.equal(await inspector.locator('[aria-label="소스 위치"]').count(), 0);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ actualReactCallsitesDistinct: true, hmrRetainsUniqueSourceAndRecomputesCss: true, classOnlyEditPreservesInspectorDocument: true, changedMetaReloadRetainsSelection: true, ambiguousSourceClearsSelection: true }));
} finally {
  const results = await Promise.allSettled([browser?.close(), server?.close(), ...created.map((file) => unlink(file))]);
  const failures = results.filter((entry) => entry.status === "rejected");
  if (failures.length) throw new AggregateError(failures.map((entry) => entry.reason), "Design write smoke cleanup failed");
}

import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { readFile, writeFile, unlink } from "node:fs/promises";
import { resolve } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright";

const slug = `dojang-inspector-${randomUUID()}`;
const file = resolve(`src/design/screens/${slug}.screen.tsx`);
const contents = `import type { CSSProperties } from "react";
export const meta = {title:"인스펙터 검증",viewport:{width:640,height:640},states:{정상:{active:true},비활성:{active:false}}};
export default function Screen({active}: {active:boolean}) {
 return <section className="bg-bg p-4 text-text">
  <div data-testid="static" className="p-4 rounded-md bg-surface text-sm font-medium" style={{width:200,height:100}}>정적 요소</div>
  <div data-testid="dynamic" className={active ? "p-[13px] text-[23px] rounded-[7px]" : "p-2"}>동적 요소</div>
  <article data-testid="ancestor" className="p-4" dangerouslySetInnerHTML={{__html:'<span data-testid="untagged">태그 없는 요소</span>'}} />
  <div data-testid="vars" className="bg-surface p-4" style={{"--c-surface":"#112233",width:220,height:120} as CSSProperties}><span style={{"--c-surface":"#ff0000",display:"block"} as CSSProperties}>자식의 다른 변수</span></div>
 </section>;
}`;
let created = false;
let server;
let browser;
const errors = [];
const evidence = {};
const hash = (text) => createHash("sha256").update(text).digest("hex");
try {
  await writeFile(file, contents, { flag: "wx" }); created = true;
  server = await createServer({ mode: "design", server: { host: "127.0.0.1", port: 0, strictPort: true }, clearScreen: false });
  await server.listen();
  const port = server.httpServer.address().port;
  browser = await chromium.launch({ headless: true, channel: "chrome" });
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${port}/design.html`);
  await page.getByRole("button", { name: "인스펙터 검증", exact: true }).click();
  const frame = page.frameLocator("iframe");
  const inspector = page.locator('[aria-label="인스펙터"]');
  const choose = async (testId) => {
    await page.getByRole("button", { name: "요소 선택", exact: true }).click();
    await frame.getByTestId(testId).click({ position: { x: 3, y: 3 } });
    await inspector.locator('[aria-label="소스 위치"]').waitFor();
  };
  await frame.getByTestId("static").waitFor();
  const source = await frame.getByTestId("static").getAttribute("data-praxis-src");
  assert.match(source, new RegExp(`src/design/screens/${slug}\\.screen\\.tsx:\\d+:\\d+`));
  assert.equal(await frame.getByTestId("static").getAttribute("data-praxis-class-kind"), "static");
  await choose("static");
  assert.equal(await inspector.locator('[aria-label="소스 위치"]').textContent(), source);
  assert.match(await inspector.locator('[aria-label="요소 크기"]').textContent(), /W 200\.0 · H 100\.0/);
  const staticInfo = await inspector.textContent();
  for (const expected of ["spacing.4", "rounded.md", "colors.surface", "typography.fontSize.sm", "typography.fontWeight.medium", "margin", "padding", "className"]) assert.ok(staticInfo.includes(expected), `missing inspector field ${expected}`);
  evidence.staticMetadataAndTokens = true;
  await choose("dynamic");
  assert.ok((await inspector.textContent()).includes("동적 className · 읽기 전용"));
  assert.ok((await inspector.textContent()).includes("토큰 밖 값"));
  evidence.dynamicReadonlyAndOutsideToken = true;
  await page.getByLabel("배율", { exact: true }).selectOption("50");
  await choose("static");
  assert.match(await inspector.locator('[aria-label="요소 크기"]').textContent(), /W 200\.0 · H 100\.0/);
  evidence.zoomKeepsCssPixels = true;
  await choose("untagged");
  assert.ok((await inspector.locator('[aria-label="소스 위치"]').textContent()).includes("소스 태그가 없습니다"));
  await inspector.getByRole("button", { name: /가까운 조상 선택/ }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="소스 위치"]')?.textContent?.includes(".screen.tsx"));
  assert.equal(await inspector.locator('[aria-label="소스 위치"]').textContent(), await frame.getByTestId("ancestor").getAttribute("data-praxis-src"));
  evidence.ancestorRequiresExplicitChoice = true;
  await choose("vars");
  assert.ok((await inspector.textContent()).includes("colors.surface"));
  evidence.selectedElementVariables = true;
  await page.getByLabel("뷰포트", { exact: true }).selectOption("390x844");
  assert.equal(await inspector.locator('[aria-label="소스 위치"]').count(), 0);
  await choose("static");
  assert.match(await inspector.locator('[aria-label="요소 크기"]').textContent(), /W 200\.0 · H 100\.0/);
  evidence.viewportClearsStaleGeometry = true;
  assert.equal(await inspector.locator("input,textarea").count(), 0);
  assert.equal(hash(await readFile(file, "utf8")), hash(contents));
  evidence.noInputsOrSourceWrites = true;
  await page.screenshot({ path: "/tmp/dojang-design-inspector-smoke.png" });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify(evidence, null, 2));
} finally {
  const cleanup = await Promise.allSettled([
    created ? unlink(file) : Promise.resolve(), browser?.close(), server?.close(),
  ]);
  const failed = cleanup.filter((entry) => entry.status === "rejected");
  if (failed.length) throw new AggregateError(failed.map((entry) => entry.reason), "Inspector smoke cleanup failed");
}

import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/sidebar-group-colors-smoke");
const fixture = path.join(directory, "ui-fixture");
const storageKey = "praxis-project-groups";
const alpha = "/workspace/alpha";
const beta = "/workspace/beta";
const gamma = "/workspace/gamma";

const initialGroups = {
  version: 1,
  groups: [
    { id: "alpha", name: "알파 그룹", collapsed: false },
    { id: "beta", name: "베타 하위", collapsed: false, parentId: "alpha" },
    { id: "gamma", name: "중립 그룹", collapsed: false },
  ],
  assignment: { [alpha]: "alpha", [beta]: "beta", [gamma]: "gamma" },
};

await mkdir(fixture, { recursive: true });
await writeFile(
  path.join(fixture, "index.html"),
  '<link rel="icon" href="data:,"><div id="root"></div><script type="module" src="./main.tsx"></script>',
);
await writeFile(
  path.join(fixture, "main.tsx"),
  `
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { SessionTaskNavigation } from "/src/components/ide/SessionTaskNavigation";
import { SessionTaskGroup } from "/src/components/ide/SessionTaskGroup";
import { loadProjectGroups, saveProjectGroups } from "/src/lib/project-groups";
import { applyTheme } from "/src/lib/themes";
import "/src/index.css";

const alpha = ${JSON.stringify(alpha)}, beta = ${JSON.stringify(beta)}, gamma = ${JSON.stringify(gamma)};
applyTheme("praxis-light", false);
(window as Window & { setFixtureTheme: (id: string) => void }).setFixtureTheme = (id) => applyTheme(id, false);
const task = (id, repo, instruction, state) => ({
  id, host: "local", repo, branch: "main", base: "main", worktree_path: repo,
  instruction, state, created_at: id, updated_at: id, mode: "conversation",
});
function Navigation() {
  const [groups, setGroups] = useState(loadProjectGroups);
  const change = (next) => { saveProjectGroups(next); setGroups(next); };
  return <SessionTaskNavigation
    tasks={[
      task(101, alpha, "alpha selected", "Running"),
      task(102, beta, "beta running", "AwaitingReview"),
      task(103, gamma, "gamma queued", "Queued"),
    ]}
    selectedKey="local:101"
    projects={[alpha, beta, gamma]}
    onOpenTask={() => {}} onNewInRepo={() => {}} onDeleteTask={() => {}}
    onRemoveProject={() => {}} onDiscardOrphans={() => {}}
    groups={groups} onProjectGroupsChange={change}
  />;
}
createRoot(document.getElementById("root")).render(
  <main id="fixture" className="min-h-screen bg-bg text-text p-3" style={{ width: 520 }}>
    <button aria-label="키보드 시작" className="sr-only">키보드 시작</button>
    <Navigation />
    <section aria-label="드롭 강조 미리보기" className="mt-5">
      <div className="mb-1 text-xs text-text-muted">드롭 강조</div>
      <SessionTaskGroup
        group={{ id: "drop-preview", name: "드롭 대상", collapsed: false, color: "rose" }}
        count={1} hasChildren={false} depth={0} path="드롭 대상" caretBefore={false} renaming={false} highlighted
        onToggle={() => {}} onRename={() => {}} onCancelRename={() => {}}
        onOpenMenu={() => {}} onDragStart={() => {}} onDragEnd={() => {}}
        onDragOver={() => {}} onHeaderDragOver={() => {}} onDragLeave={() => {}} onDrop={() => {}} onHeaderDrop={() => {}}
      >
        <div className="mx-2 mb-1 rounded-md border border-border bg-surface px-2 py-1 text-xs text-text-secondary">프로젝트 드롭 대상</div>
      </SessionTaskGroup>
    </section>
  </main>,
);
`,
);

let server;
let browser;
try {
  server = await createServer({
    configFile: false,
    plugins: [react()],
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  browser = await chromium.launch({
    headless: true,
    channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome",
  });
  const page = await browser.newPage({ viewport: { width: 640, height: 900 } });
  const pageErrors = [];
  const consoleErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  const url = `${server.resolvedUrls.local[0]}${path.relative(process.cwd(), fixture)}/index.html`;

  const header = (id) => page.locator(`[data-group-box="${id}"]`).locator(":scope > [data-group-header]");
  const groupBox = (id) => page.locator(`[data-group-box="${id}"]`);
  const groupBody = (id) => groupBox(id).locator(":scope > [data-group-body]");
  const colorMenu = () => page.getByRole("menu", { name: "그룹 색상" });
  const saved = () => page.evaluate((key) => JSON.parse(localStorage.getItem(key)), storageKey);
  const savedGroup = async (id) => (await saved()).groups.find((group) => group.id === id);
  const parentId = async (id) => (await savedGroup(id))?.parentId ?? null;
  const seed = async (groups) => {
    await page.goto(url);
    await page.evaluate(([key, value]) => localStorage.setItem(key, JSON.stringify(value)), [storageKey, groups]);
    await page.reload();
    await header("alpha").waitFor();
  };
  const menuBounds = async (menu) => {
    const box = await menu.boundingBox();
    const viewport = page.viewportSize();
    assert(box && viewport, "menu should have a visible bounding box");
    assert(box.x >= 8 && box.y >= 8, `menu must keep an 8px inset: ${JSON.stringify(box)}`);
    assert(
      box.x + box.width <= viewport.width - 8 && box.y + box.height <= viewport.height - 8,
      `menu must stay within viewport: ${JSON.stringify(box)}`,
    );
  };
  const mouseDrag = async (source, target, edge = false) => {
    await source.scrollIntoViewIfNeeded();
    const from = await source.boundingBox();
    if (!("x" in target)) await target.scrollIntoViewIfNeeded();
    const to = "x" in target ? target : await target.boundingBox();
    assert(from && to, "drag source and destination must be visible");
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2 + 2, from.y + from.height / 2 + 2, { steps: 2 });
    await page.mouse.move(to.x + to.width / 2, edge ? to.y + 2 : to.y + to.height / 2, { steps: 10 });
    await page.mouse.up();
  };
  const isFocused = (locator) => locator.evaluate((node) => document.activeElement === node);
  const tabTo = async (locator, description) => {
    for (let count = 0; count < 20; count += 1) {
      if (await isFocused(locator)) return;
      await page.keyboard.press("Tab");
    }
    assert.fail(`Tab did not reach ${description}`);
  };
  const openGroupMenuWithKeyboard = async (id) => {
    await header(id).focus();
    await page.keyboard.press("Shift+F10");
    const menu = page.getByRole("menu").last();
    await menu.waitFor();
    return menu;
  };
  const openColorMenuWithKeyboard = async (id) => {
    const rootMenu = await openGroupMenuWithKeyboard(id);
    const groupColor = rootMenu.getByRole("button", { name: "그룹 색상" });
    await tabTo(groupColor, "the group color action");
    assert.equal(await isFocused(groupColor), true, "Tab focuses the group color action");
    await page.keyboard.press("Enter");
    const menu = colorMenu();
    await menu.waitFor();
    assert.equal(
      await isFocused(menu.getByRole("menuitemradio", { name: "파랑" })),
      true,
      "opening the color page focuses its first choice",
    );
    return menu;
  };
  const selectColorWithKeyboard = async (id, label) => {
    const menu = await openColorMenuWithKeyboard(id);
    const item = menu.getByRole("menuitemradio", { name: label });
    await tabTo(item, `${label} color choice`);
    assert.equal(await isFocused(item), true, `Tab focuses ${label}`);
    await page.keyboard.press("Enter");
    await menu.waitFor({ state: "hidden" });
  };
  const applyFixtureTheme = async (id) => {
    await page.evaluate((themeId) => window.setFixtureTheme(themeId), id);
    // Header uses transition-colors, so wait for its final token value rather than sampling mid-transition.
    await page.waitForFunction(() => {
      const header = document.querySelector("[data-group-header]");
      if (header == null) return false;
      const probe = document.createElement("span");
      probe.className = "text-text";
      probe.style.position = "absolute";
      probe.style.visibility = "hidden";
      document.body.append(probe);
      const expected = getComputedStyle(probe).color;
      probe.remove();
      return getComputedStyle(header).color === expected;
    });
  };
  const groupVisual = (id) =>
    page.locator(`[data-group-box="${id}"]`).evaluate((box) => {
      const header = box.querySelector(":scope > [data-group-header]");
      const body = box.querySelector(":scope > [data-group-body]");
      const strip = box.querySelector(":scope > [data-group-color-strip]");
      const taskLabel = Array.from(box.querySelectorAll("span")).find((node) =>
        node.textContent === "alpha selected",
      );
      const taskCard = taskLabel?.closest('div[class*="cursor-pointer"]');
      const statusDot = box.querySelector('[role="img"]');
      const style = (node) => (node ? getComputedStyle(node) : null);
      return {
        headerBackground: style(header)?.backgroundColor,
        headerInlineBackground: header?.style.backgroundColor,
        stripWidth: style(strip)?.width,
        bodyBackground: style(body)?.backgroundColor,
        boxBackground: style(box)?.backgroundColor,
        boxBorder: style(box)?.borderColor,
        taskBackground: style(taskCard)?.backgroundColor,
        statusDot: style(statusDot)?.backgroundColor,
      };
    });
  const contrastForHeader = (id) =>
    page.locator(`[data-group-box="${id}"] > [data-group-header]`).evaluate((header) => {
      const rgb = (value) => {
        if (!CSS.supports("color", value)) throw new Error(`Unsupported computed color: ${value}`);
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 1;
        const context = canvas.getContext("2d");
        if (context == null) throw new Error("Canvas 2D context is unavailable");
        context.clearRect(0, 0, 1, 1);
        context.fillStyle = value;
        context.fillRect(0, 0, 1, 1);
        const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
        return [red, green, blue, alpha / 255];
      };
      const compose = (foreground, background) => {
        const alpha = foreground[3] + background[3] * (1 - foreground[3]);
        return alpha === 0
          ? [0, 0, 0, 0]
          : [0, 1, 2].map((index) =>
              (foreground[index] * foreground[3] + background[index] * background[3] * (1 - foreground[3])) / alpha,
            ).concat(alpha);
      };
      const background = (node) => {
        if (node == null) return [255, 255, 255, 1];
        return compose(rgb(getComputedStyle(node).backgroundColor), background(node.parentElement));
      };
      const luminance = (color) => {
        const channels = color.slice(0, 3).map((value) => {
          const normalized = value / 255;
          return normalized <= 0.03928 ? normalized / 12.92 : ((normalized + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
      };
      const foreground = compose(rgb(getComputedStyle(header).color), [0, 0, 0, 0]);
      const ratio = (Math.max(luminance(foreground), luminance(background(header))) + 0.05) /
        (Math.min(luminance(foreground), luminance(background(header))) + 0.05);
      return {
        ratio,
        text: getComputedStyle(header).color,
        tint: getComputedStyle(header).backgroundColor,
        token: getComputedStyle(document.documentElement).getPropertyValue("--c-text").trim(),
      };
    });

  const minimumContrast = { light: Infinity, dark: Infinity };

  // R-1 / Q-3: a prior fixture control reaches the header by Tab, then keyboard-only menu flow reaches named presets.
  await seed(initialGroups);
  await page.setViewportSize({ width: 320, height: 360 });
  await page.getByRole("button", { name: "키보드 시작" }).focus();
  await page.keyboard.press("Tab");
  assert.equal(await isFocused(header("alpha")), true, "Tab reaches the first group header from the preceding control");
  let menu = await openGroupMenuWithKeyboard("alpha");
  await menuBounds(menu);
  const colorAction = menu.getByRole("button", { name: "그룹 색상" });
  await tabTo(colorAction, "the group color action");
  assert.equal(await isFocused(colorAction), true, "Tab reaches the color action in the root menu");
  await page.keyboard.press("Enter");
  menu = colorMenu();
  await menu.waitFor();
  await menuBounds(menu);
  assert.deepEqual(
    await menu.getByRole("menuitemradio").allTextContents(),
    ["파랑", "보라", "분홍", "주황", "초록", "회색", "기본값으로 되돌리기"],
  );
  const beforeEscape = await saved();
  await page.keyboard.press("Escape");
  await colorMenu().waitFor({ state: "hidden" });
  assert.deepEqual(await saved(), beforeEscape, "Escape must not change the saved color");
  assert.equal(await header("alpha").evaluate((node) => document.activeElement === node), true, "Escape restores header focus");

  await selectColorWithKeyboard("alpha", "파랑");
  await selectColorWithKeyboard("beta", "초록");
  let persisted = await saved();
  assert.equal(persisted.groups.find((group) => group.id === "alpha")?.color, "blue");
  assert.equal(persisted.groups.find((group) => group.id === "beta")?.color, "green");
  assert.deepEqual(persisted.assignment, initialGroups.assignment, "colors leave project assignments intact");

  // R-2: ordinary group interactions retain the color property and storage round-trip.
  await page.setViewportSize({ width: 640, height: 900 });
  await header("alpha").click({ button: "right" });
  await page.getByRole("menu").getByRole("button", { name: "이름 바꾸기" }).click();
  const nameInput = page.getByLabel("그룹 이름");
  await nameInput.fill("바뀐 알파");
  await nameInput.press("Enter");
  await header("alpha").click();
  assert.equal(await groupBody("alpha").count(), 0, "group collapses");
  assert.equal(await groupBox("beta").count(), 0, "parent collapse hides its colored child");
  const collapsedBox = await groupBox("alpha").boundingBox();
  const collapsedStrip = await groupBox("alpha").locator(":scope > [data-group-color-strip]").boundingBox();
  assert(collapsedBox && collapsedStrip, "collapsed colored group retains its strip");
  assert.equal(Math.round(collapsedBox.height - collapsedStrip.height), 8, "the strip spans the collapsed group edge, inset only for its rounded ends");
  await header("alpha").click();
  const orderBeforeReorder = persisted.groups.map((group) => group.id);
  await mouseDrag(header("gamma"), header("alpha"), true);
  persisted = await saved();
  assert.equal(persisted.groups.find((group) => group.id === "alpha")?.name, "바뀐 알파");
  assert.equal(persisted.groups.find((group) => group.id === "alpha")?.color, "blue");
  assert.equal(persisted.groups.find((group) => group.id === "beta")?.color, "green");
  assert.deepEqual(persisted.groups.map((group) => group.id), ["gamma", "alpha", "beta"], "header-edge drag reorders root siblings without nesting");
  assert.notDeepEqual(persisted.groups.map((group) => group.id), orderBeforeReorder, "the persisted order actually changed");
  assert.equal(await parentId("beta"), "alpha", "reorder retains the child link");
  await page.reload();
  await header("alpha").waitFor();
  assert.deepEqual(await saved(), persisted, "reload keeps renamed, toggled, reordered color state");

  // R-4 / Q-2: the 3px strip spans the group edge, tint stays in the header, and drop treatment overrides both.
  const neutralBefore = await groupVisual("gamma");
  const coloredBefore = await groupVisual("alpha");
  assert.equal(coloredBefore.stripWidth, "3px", "a colored group has a 3px strip");
  assert.notEqual(coloredBefore.headerBackground, neutralBefore.headerBackground, "only the colored header receives tint");
  assert.equal(coloredBefore.bodyBackground, neutralBefore.bodyBackground, "group bodies retain their normal background");
  const selectedBeforeDrop = { task: coloredBefore.taskBackground, dot: coloredBefore.statusDot };
  assert.equal(typeof selectedBeforeDrop.task, "string", "the selected task card was found");
  assert.equal(typeof selectedBeforeDrop.dot, "string", "the selected task status dot was found");
  await groupBody("beta").locator('[draggable="true"]').evaluate((node) => {
    const data = new DataTransfer();
    node.dispatchEvent(new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer: data }));
    const target = document.querySelector('[data-group-box="alpha"]');
    target?.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
  });
  await page.waitForFunction(() => document.querySelector('[data-group-box="alpha"]')?.className.includes("border-primary"));
  const dropped = await groupVisual("alpha");
  assert.notEqual(dropped.boxBackground, "rgba(0, 0, 0, 0)", "drop target has a visible background");
  assert.notEqual(dropped.boxBorder, "rgba(0, 0, 0, 0)", "drop target keeps a visible outline");
  assert.equal(dropped.headerInlineBackground, "", "drop treatment yields to the group tint");
  assert.deepEqual(
    { task: dropped.taskBackground, dot: dropped.statusDot },
    selectedBeforeDrop,
    "drop styling leaves selected task and status visuals unchanged",
  );
  await groupBox("alpha").evaluate((target) => {
    const data = new DataTransfer();
    data.setData("application/x-praxis-project", "/workspace/beta");
    target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
  });
  await page.waitForFunction(() => !document.querySelector('[data-group-box="alpha"]')?.className.includes("border-primary"));

  // Q-1: every named preset keeps the neutral header text readable in both shipped schemes.
  for (const dark of [false, true]) {
    await applyFixtureTheme(dark ? "praxis-dark" : "praxis-light");
    for (const label of ["파랑", "보라", "분홍", "주황", "초록", "회색"]) {
      await selectColorWithKeyboard("alpha", label);
      const contrast = await contrastForHeader("alpha");
      const ratio = contrast.ratio;
      minimumContrast[dark ? "dark" : "light"] = Math.min(minimumContrast[dark ? "dark" : "light"], ratio);
      assert(
        ratio >= 4.5,
        `${label} header text needs WCAG AA contrast in ${dark ? "dark" : "light"} mode (measured ${ratio.toFixed(2)}:1; text ${contrast.text}; tint ${contrast.tint}; --c-text ${contrast.token})`,
      );
    }
    await selectColorWithKeyboard("alpha", "파랑");
    await selectColorWithKeyboard("beta", "초록");
    await selectColorWithKeyboard("gamma", "주황");
    await page.screenshot({ path: path.join(directory, dark ? "dark.png" : "light.png"), fullPage: true });
  }

  // Nested + colors: moving a child, dissolving its colored parent, reset, and reload preserve descendant links and colors.
  const nestedColored = structuredClone(initialGroups);
  nestedColored.groups.find((group) => group.id === "alpha").color = "blue";
  nestedColored.groups.find((group) => group.id === "beta").color = "green";
  nestedColored.groups.find((group) => group.id === "gamma").color = "orange";
  await seed(nestedColored);
  await page.setViewportSize({ width: 320, height: 900 });
  await page.locator("main").evaluate((node) => { node.style.width = "240px"; node.style.height = "850px"; });
  assert.equal(await page.locator("main").evaluate((node) => node.scrollWidth > node.clientWidth), false, "nested colored strips fit a 240px sidebar");
  assert.equal(await groupBox("alpha").locator(":scope > [data-group-color-strip]").count(), 1, "colored parent owns its strip");
  assert.equal(await groupBox("beta").locator(":scope > [data-group-color-strip]").count(), 1, "colored child owns its own strip");
  await mouseDrag(header("beta"), page.locator("[data-unassigned]"));
  assert.equal(await parentId("beta"), null, "child can move to root");
  assert.equal((await savedGroup("beta")).color, "green", "moving child retains its color");
  assert.equal((await saved()).assignment[beta], "beta", "moving child retains its project link");
  await mouseDrag(header("beta"), header("alpha"));
  assert.equal(await parentId("beta"), "alpha", "child can move back under colored parent");
  await header("alpha").click({ button: "right" });
  await page.getByRole("menu").getByRole("button", { name: /그룹 해제/ }).click();
  let promoted = await saved();
  assert.equal(promoted.groups.some((group) => group.id === "alpha"), false, "dissolve removes only the parent");
  assert.equal(await parentId("beta"), null, "dissolve promotes the child to root");
  assert.equal(promoted.groups.find((group) => group.id === "beta")?.color, "green", "promotion keeps child color");
  assert.equal(promoted.groups.find((group) => group.id === "gamma")?.color, "orange", "unrelated group color survives dissolve");
  assert.equal(promoted.assignment[beta], "beta", "promotion keeps the child project link");
  await selectColorWithKeyboard("beta", "기본값으로 되돌리기");
  promoted = await saved();
  assert.equal("color" in promoted.groups.find((group) => group.id === "beta"), false, "reset removes only promoted child color");
  assert.equal(promoted.groups.find((group) => group.id === "gamma")?.color, "orange", "reset leaves sibling color intact");
  await page.reload();
  await header("beta").waitFor();
  assert.deepEqual(await saved(), promoted, "nested color move, promotion, reset, and links survive reload");

  // R-3: no-color legacy data, reset, and an invalid stored color preserve the group and assignment.
  await applyFixtureTheme("praxis-light");
  await seed(initialGroups);
  await selectColorWithKeyboard("alpha", "보라");
  await openColorMenuWithKeyboard("alpha");
  const reset = colorMenu().getByRole("menuitemradio", { name: "기본값으로 되돌리기" });
  await tabTo(reset, "the reset choice");
  assert.equal(await isFocused(reset), true, "Tab reaches reset");
  await page.keyboard.press("Enter");
  persisted = await saved();
  assert.equal("color" in persisted.groups.find((group) => group.id === "alpha"), false, "reset removes the optional property");
  assert.deepEqual(persisted.assignment, initialGroups.assignment, "reset preserves assignments");

  const legacy = structuredClone(initialGroups);
  await seed(legacy);
  assert.equal(await groupBox("alpha").locator(":scope > [data-group-color-strip]").count(), 0, "legacy colorless groups render neutral");
  const invalid = structuredClone(initialGroups);
  invalid.groups[0].color = "not-a-preset";
  await seed(invalid);
  assert.equal(await groupBox("alpha").locator(":scope > [data-group-color-strip]").count(), 0, "invalid saved color falls back to neutral");
  await header("alpha").click({ button: "right" });
  await page.getByRole("menu").getByRole("button", { name: "이름 바꾸기" }).click();
  await page.getByLabel("그룹 이름").fill("정상화된 알파");
  await page.getByLabel("그룹 이름").press("Enter");
  persisted = await saved();
  assert.equal("color" in persisted.groups.find((group) => group.id === "alpha"), false, "the next save omits invalid color data");
  assert.deepEqual(persisted.assignment, initialGroups.assignment, "invalid color never drops assignments");

  assert.deepEqual(pageErrors, [], `browser page errors: ${pageErrors.join(" | ")}`);
  assert.deepEqual(consoleErrors, [], `browser console errors: ${consoleErrors.join(" | ")}`);
  console.log(`PASS sidebar group colors: R1-R4, Q1-Q3; min contrast light ${minimumContrast.light.toFixed(2)}:1, dark ${minimumContrast.dark.toFixed(2)}:1; light/dark screenshots written`);
} finally {
  await browser?.close();
  await server?.close();
  await rm(fixture, { recursive: true, force: true });
}

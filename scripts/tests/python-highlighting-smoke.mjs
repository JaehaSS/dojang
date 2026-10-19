import assert from "node:assert/strict";
import { mkdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import react from "@vitejs/plugin-react";

const directory = path.resolve(".dojang/verification/python-highlighting-smoke");
const fixture = path.join(directory, "ui-fixture");
const source = (file) => `/@fs/${path.resolve(file)}`;
const darkScreenshot = process.env.PRAXIS_PYTHON_HIGHLIGHTING_DARK_SCREENSHOT || "/tmp/praxis-python-highlighting-dark.png";
const lightScreenshot = process.env.PRAXIS_PYTHON_HIGHLIGHTING_LIGHT_SCREENSHOT || "/tmp/praxis-python-highlighting-light.png";
const python = [
  "# fake def fake_method() and class Fake must stay a comment",
  "class BootstrapTests:",
  "    def setUp(self):",
  '        self.archive = "def fake_method() and class Fake"',
  "        self.write_archive()",
].join("\n");

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
import Editor from "@monaco-editor/react";
import { applyTheme } from "${source("src/lib/themes.ts")}";
import { langFromPath, monaco } from "${source("src/lib/monaco.ts")}";

const initialFiles = {
  "preview/bootstrap.py": ${JSON.stringify(python)},
  "types/bootstrap.pyi": ${JSON.stringify(python)},
  "preview/alternate.pyi": ${JSON.stringify(python)},
  "work/bootstrap.py": ${JSON.stringify(python)},
};

applyTheme("night-owl", false);

const normalize = (value) => value.replaceAll(String.fromCharCode(160), " ");
window.samplePythonColors = (surface, includeMethodCall) => {
  const root = document.querySelector(
    '[data-surface="' + surface + '"]',
  );
  const color = (lineText, tokenText) => {
    const line = Array.from(root?.querySelectorAll(".view-line") || []).find(
      (candidate) => normalize(candidate.textContent || "").includes(lineText),
    );
    const token = Array.from(line?.querySelectorAll("span") || []).find((candidate) =>
      candidate.childElementCount === 0 && candidate.className.includes("mtk") &&
        normalize(candidate.textContent || "").includes(tokenText),
    );
    return token == null ? null : getComputedStyle(token).color;
  };
  return {
    className: color("class BootstrapTests:", "BootstrapTests"),
    functionName: color("def setUp(self):", "setUp"),
    variableName: color("self.archive =", "archive"),
    comment: color("# fake def", "# fake"),
    string: color("self.archive =", "fake_method()"),
    methodCall: includeMethodCall ? color("self.write_archive()", "write_archive") : null,
  };
};
window.dumpPythonTokens = () => Object.fromEntries(["readonly", "editable"].map((surface) => [surface,
  Array.from(document.querySelectorAll('[data-surface="' + surface + '"] .view-lines span')).map((node) => ({
    text: node.textContent,
    className: node.className,
    color: getComputedStyle(node).color,
  })),
]));

function Harness() {
  const [files, setFiles] = useState(initialFiles);
  const [theme, setTheme] = useState("night-owl");
  const [readPath, setReadPath] = useState("preview/bootstrap.py");
  const [editPath, setEditPath] = useState("types/bootstrap.pyi");
  const mounted = (name) => (editor) => {
    window.__editors = window.__editors || {};
    window.__editors[name] = editor;
    window.__mounted = window.__mounted || [];
    window.__mounted.push({ name, path: editor.getModel().uri.path });
  };
  window.__monaco = monaco;
  window.__python = initialFiles["preview/bootstrap.py"];
  window.__activeSmokeTheme = theme;
  window.setSmokeTheme = (id) => { applyTheme(id, false); setTheme(id); };
  return <main>
    <button onClick={() => { setReadPath("preview/alternate.pyi"); setEditPath("work/bootstrap.py"); }}>switch files</button>
    <output data-read-path>{readPath}</output><output data-edit-path>{editPath}</output>
    <section data-surface="readonly"><Editor height="210px" path={readPath} value={files[readPath]} language={langFromPath(readPath)} theme={theme} onMount={mounted("readonly")} options={{ readOnly: true, minimap: { enabled: false }, scrollBeyondLastLine: false }} /></section>
    <section data-surface="editable"><Editor height="210px" path={editPath} value={files[editPath]} language={langFromPath(editPath)} theme={theme} onMount={mounted("editable")} onChange={(value) => setFiles((current) => ({ ...current, [editPath]: value || "" }))} options={{ minimap: { enabled: false }, scrollBeyondLastLine: false }} /></section>
  </main>;
}

createRoot(document.getElementById("root")).render(<Harness />);
`,
);

let server;
let browser;
try {
  server = await createServer({
    configFile: false,
    root: fixture,
    plugins: [react()],
    server: { host: "127.0.0.1", port: 0 },
    logLevel: "error",
  });
  await server.listen();
  browser = await chromium.launch({
    headless: true,
    channel: process.env.PRAXIS_SMOKE_BROWSER || "chrome",
  });
  const page = await browser.newPage({ viewport: { width: 900, height: 520 } });
  const pageErrors = [];
  const consoleErrors = [];
  page.on("pageerror", (error) => pageErrors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  await page.goto(`${server.resolvedUrls.local[0]}index.html`);
  await page.waitForFunction(() => window.__mounted?.length === 2 && window.__editors?.editable != null);

  const waitForColors = async (previous, editableMethodCall) => {
    try {
      const handle = await page.waitForFunction((options) => {
        const colors = {
          readonly: window.samplePythonColors("readonly", true),
          editable: window.samplePythonColors("editable", options.editableMethodCall),
        };
        const present = (sample, requireMethodCall) => Object.entries(sample).every(
          ([key, value]) => (key === "methodCall" && !requireMethodCall) || typeof value === "string",
        );
        const changed = options.previous == null ||
          (colors.readonly.className !== options.previous.readonly && colors.editable.className !== options.previous.editable);
        return present(colors.readonly, true) && present(colors.editable, options.editableMethodCall) && changed ? colors : null;
      }, { previous, editableMethodCall }, { timeout: 2_000 });
      return await handle.jsonValue();
    } catch (error) {
      await page.screenshot({ path: "/tmp/praxis-python-highlighting-timeout.png", fullPage: true });
      console.error(`Python highlighting color-sampler diagnostic: ${JSON.stringify(await page.evaluate(() => ({
        samples: {
          readonly: window.samplePythonColors("readonly", true),
          editable: window.samplePythonColors("editable", false),
        },
        tokens: window.dumpPythonTokens(),
      })))}`);
      throw error;
    }
  };
  const assertSyntaxColors = (observed, description) => {
    assert.notEqual(observed.className, observed.functionName, `${description}: class and function colors differ`);
    assert.notEqual(observed.functionName, observed.variableName, `${description}: function and variable colors differ`);
    assert.notEqual(observed.className, observed.variableName, `${description}: class and variable colors differ`);
    assert.notEqual(observed.comment, observed.className, `${description}: comments retain comment color`);
    assert.notEqual(observed.string, observed.functionName, `${description}: strings retain string color`);
    if (observed.methodCall != null) {
      assert.equal(observed.methodCall, observed.functionName, `${description}: method calls retain function color`);
      assert.notEqual(observed.methodCall, observed.variableName, `${description}: method calls differ from variables`);
    }
  };
  const tokenizerTypes = await page.evaluate(() => {
    return window.__monaco.editor.tokenize(window.__python, "python").map((tokens) =>
      tokens.map((token) => token.type),
    );
  });
  assert(tokenizerTypes.flat().some((type) => /comment/.test(type)), "the Python tokenizer emits a comment token");
  assert(tokenizerTypes.flat().some((type) => /string/.test(type)), "the Python tokenizer emits a string token");
  assert.equal(await page.locator("[data-read-path]").textContent(), "preview/bootstrap.py");
  assert.equal(await page.locator("[data-edit-path]").textContent(), "types/bootstrap.pyi");
  assert.deepEqual(
    await page.evaluate(() => Object.values(window.__editors).map((editor) => editor.getModel().getLanguageId())),
    ["python", "python"],
    "both initial .py and .pyi surfaces use Python",
  );
  const { readonly: nightReadonly, editable: nightEditable } = await waitForColors(null, true);
  assertSyntaxColors(nightReadonly, "Night Owl readonly .py");
  assertSyntaxColors(nightEditable, "Night Owl editable .pyi");
  await page.screenshot({ path: darkScreenshot, fullPage: true });

  await page.getByRole("button", { name: "switch files" }).click();
  await page.waitForFunction(() =>
    window.__editors?.readonly?.getModel().uri.path.endsWith("preview/alternate.pyi") &&
    window.__editors?.editable?.getModel().uri.path.endsWith("work/bootstrap.py"),
  );
  assert.equal(await page.locator("[data-read-path]").textContent(), "preview/alternate.pyi");
  assert.equal(await page.locator("[data-edit-path]").textContent(), "work/bootstrap.py");
  assert.deepEqual(
    await page.evaluate(() => Object.values(window.__editors).map((editor) => editor.getModel().getLanguageId())),
    ["python", "python"],
    "file switches preserve .py and .pyi language detection",
  );
  await page.evaluate(() => {
    const editor = window.__editors.editable;
    editor.focus();
    editor.setPosition({ lineNumber: 5, column: 1 });
  });
  await page.keyboard.type("# edited ");
  assert.match(
    await page.evaluate(() => window.__editors.editable.getValue()),
    /# edited\s+self\.write_archive\(\)/,
    "typing into the editable surface changes its Monaco model",
  );
  const editedTokenTypes = await page.evaluate(() =>
    window.__monaco.editor.tokenize(window.__editors.editable.getValue(), "python")[4].map((token) => token.type),
  );
  assert(editedTokenTypes.some((type) => /comment/.test(type)), "editing a line into a comment retokenizes it as a comment");

  await page.evaluate(() => window.setSmokeTheme("praxis-light"));
  await page.waitForFunction(() => window.__activeSmokeTheme === "praxis-light");
  const { readonly: lightReadonly, editable: lightEditable } = await waitForColors(
    { readonly: nightReadonly.className, editable: nightEditable.className },
    false,
  );
  assert.notEqual(lightReadonly.className, nightReadonly.className, "Praxis Light changes the readonly class color");
  assert.notEqual(lightEditable.className, nightEditable.className, "Praxis Light changes the editable class color");
  assertSyntaxColors(lightReadonly, "Praxis Light readonly .pyi");
  assertSyntaxColors(lightEditable, "Praxis Light editable .py");
  await page.screenshot({ path: lightScreenshot, fullPage: true });
  await page.evaluate(() => window.setSmokeTheme("praxis-dark"));
  await page.waitForFunction(() => window.__activeSmokeTheme === "praxis-dark");
  const { readonly: praxisDarkReadonly, editable: praxisDarkEditable } = await waitForColors(
    { readonly: lightReadonly.className, editable: lightEditable.className },
    false,
  );
  assert.notEqual(praxisDarkReadonly.className, lightReadonly.className, "Praxis Dark changes the readonly class color");
  assert.notEqual(praxisDarkEditable.className, lightEditable.className, "Praxis Dark changes the editable class color");
  assertSyntaxColors(praxisDarkReadonly, "Praxis Dark readonly .pyi");
  assertSyntaxColors(praxisDarkEditable, "Praxis Dark editable .py");
  assert.deepEqual(pageErrors, [], `page errors: ${pageErrors.join("\\n")}`);
  assert.deepEqual(consoleErrors, [], `console errors: ${consoleErrors.join("\\n")}`);
  console.log(`PASS python highlighting: .py/.pyi read-only and editable surfaces, lazy Monaco load, switch, edit, Night Owl/Praxis Light screenshots ${darkScreenshot} ${lightScreenshot}`);
} finally {
  await browser?.close();
  await server?.close();
  await rm(directory, { recursive: true, force: true });
}

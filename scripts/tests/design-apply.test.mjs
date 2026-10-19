import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import test from "node:test";

const root = process.cwd();
const script = join(root, "scripts/design-apply.mjs");

async function invoke(request) {
  const child = spawn(process.execPath, [script], { cwd: root, stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  child.stdin.end(JSON.stringify({ root, ...request }));
  await new Promise((resolveRun, rejectRun) => {
    child.once("error", rejectRun);
    child.once("close", (code) => code === 0 ? resolveRun() : rejectRun(new Error(`design-apply exited ${code}`)));
  });
  assert.equal(stderr, "");
  return JSON.parse(stdout);
}

async function fixture(files) {
  const directory = await mkdtemp(join(root, ".dojang-design-apply-test-"));
  await Promise.all(Object.entries(files).map(async ([name, content]) => {
    const path = join(directory, name);
    await writeFile(path, content);
  }));
  return {
    directory,
    source(name, line, column) {
      return `${relative(root, join(directory, name))}:${line}:${column}`;
    },
    sourceAt(name, needle) {
      const content = files[name];
      const index = content.indexOf(needle);
      assert.notEqual(index, -1, `missing ${needle}`);
      const previous = content.lastIndexOf("\n", index);
      const line = content.slice(0, index).split("\n").length;
      return `${relative(root, join(directory, name))}:${line}:${index - previous - 1}`;
    },
  };
}

test("proposes a minimal static class replacement and never writes the file", async (t) => {
  const files = await fixture({
    "Card.tsx": "export function Card() {\n  return <div className='p-4 hover:p-6 text-sm'>x</div>;\n}\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.source("Card.tsx", 2, 9);
  const inspected = await invoke({ operation: "inspect", source, scope: "component" });
  assert.equal(inspected.ok, true);
  assert.equal(inspected.editable, true);
  assert.equal(inspected.kind, "static");
  assert.equal(inspected.shared, true);
  const applied = await invoke({ operation: "apply", source, scope: "component", property: "paddingLeft", value: "20px", expectedHash: inspected.hash });
  assert.equal(applied.ok, true);
  assert.equal(applied.previousHash, inspected.hash);
  assert.match(applied.content, /className='pt-4 pr-4 pb-4 pl-\[20px\] hover:p-6 text-sm'/);
  const untouched = await (await import("node:fs/promises")).readFile(join(files.directory, "Card.tsx"), "utf8");
  assert.match(untouched, /className='p-4 hover:p-6 text-sm'/);
});

test("uses a named relative import callsite for instance scope and rejects unsupported props as inspect metadata", async (t) => {
  const files = await fixture({
    "Card.tsx": "export interface CardProps { className?: string }\nexport function Card({ className }: CardProps) {\n  return <section className={className}>x</section>;\n}\n",
    "Screen.tsx": "import { Card } from './Card';\nexport const Screen = () => <Card className=\"p-2\" />;\n",
    "Plain.tsx": "export function Plain({ title }) { return <div>{title}</div>; }\n",
    "Other.tsx": "import { Plain } from './Plain';\nexport const Other = () => <Plain />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const inspected = await invoke({
    operation: "inspect",
    source: files.source("Card.tsx", 3, 9),
    instanceSource: files.source("Screen.tsx", 2, 28),
    scope: "instance",
  });
  assert.equal(inspected.ok, true);
  assert.equal(inspected.file, relative(root, join(files.directory, "Screen.tsx")));
  assert.equal(inspected.source, files.source("Screen.tsx", 2, 28));
  assert.equal(inspected.selectionSource, files.source("Card.tsx", 3, 9));
  assert.equal(inspected.shared, true);
  const applied = await invoke({
    operation: "apply",
    source: files.source("Card.tsx", 3, 9),
    instanceSource: files.source("Screen.tsx", 2, 28),
    scope: "instance",
    property: "background-color",
    value: "#123456",
    expectedHash: inspected.hash,
  });
  assert.match(applied.content, /className="p-2 bg-\[#123456\]"/);

  const unsupported = await invoke({
    operation: "inspect",
    source: files.source("Plain.tsx", 1, 42),
    instanceSource: files.source("Other.tsx", 2, 27),
    scope: "instance",
  });
  assert.deepEqual({ ok: unsupported.ok, editable: unsupported.editable, reason: unsupported.reason }, { ok: true, editable: false, reason: "no_classname_prop" });
});

test("supports default imports and rejects dynamic class data, stale content, and mismatched callsites", async (t) => {
  const files = await fixture({
    "Default.tsx": "export default function Default({ className }) { return <div className={className}>x</div>; }\n",
    "UsesDefault.tsx": "import Default from './Default';\nexport const UsesDefault = () => <Default />;\n",
    "Dynamic.tsx": "export const Dynamic = () => <div {...props} className={ready ? 'p-1' : 'p-2'} />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const instance = await invoke({ operation: "inspect", source: files.source("Default.tsx", 1, 56), instanceSource: files.source("UsesDefault.tsx", 2, 33), scope: "instance" });
  assert.equal(instance.editable, true);
  assert.equal(instance.kind, "none");
  assert.equal(instance.className, "");
  const dynamic = await invoke({ operation: "inspect", source: files.source("Dynamic.tsx", 1, 29), scope: "component" });
  assert.deepEqual({ ok: dynamic.ok, editable: dynamic.editable, reason: dynamic.reason }, { ok: true, editable: false, reason: "dynamic_classname" });
  assert.equal(dynamic.className, "");
  const stale = await invoke({ operation: "apply", source: files.source("Dynamic.tsx", 1, 29), scope: "component", property: "width", value: "1px", expectedHash: "old" });
  assert.equal(stale.reason, "stale_file");
  const invalid = await invoke({ operation: "inspect", source: files.source("Default.tsx", 1, 56), instanceSource: files.source("Dynamic.tsx", 1, 29), scope: "instance" });
  assert.equal(invalid.reason, "invalid_instance");
});

test("proves local component callsites before selecting an instance target", async (t) => {
  const files = await fixture({
    "Local.tsx": "function Chip({ className }) {\n  return <span className={className}>chip</span>;\n}\nexport const Screen = () => <Chip className=\"rounded-md\" />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const response = await invoke({
    operation: "inspect",
    source: files.source("Local.tsx", 2, 9),
    instanceSource: files.source("Local.tsx", 4, 28),
    scope: "instance",
  });
  assert.equal(response.ok, true);
  assert.equal(response.editable, true);
  assert.equal(response.source, files.source("Local.tsx", 4, 28));
});

test("preserves unrelated Tailwind families and raw JSX whitespace while editing static classes", async (t) => {
  const files = await fixture({
    "Families.tsx": "export const Families = () => <div className=\"p-4 pl-6\ttext-left\nbg-gradient-to-r bg-[url(hero.png)] bg-opacity-50 text-[14px] font-[Arial] foo&amp;bar\" />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Families.tsx", "<div");
  const inspected = await invoke({ operation: "inspect", source, scope: "component" });
  const applied = await invoke({ operation: "apply", source, scope: "component", property: "padding-left", value: "13px", expectedHash: inspected.hash });
  assert.match(applied.content, /pt-4 pr-4 pb-4 pl-\[13px\]\ttext-left\nbg-gradient-to-r bg-\[url\(hero\.png\)\] bg-opacity-50 text-\[14px\] font-\[Arial\] foo&amp;bar/);
  const unsafe = await invoke({ operation: "apply", source, scope: "component", property: "width", value: "calc(100% - 4px) \"", expectedHash: inspected.hash });
  assert.equal(unsafe.reason, "unsafe_value");
});

test("adds missing classes, handles geometry/color families, and keeps default instance selection unshared", async (t) => {
  const files = await fixture({
    "Plain.screen.tsx": "export default () => <div className=\"w-4 h-3 rounded-md bg-red-500 text-blue-500 text-center text-sm font-bold font-[Arial]\" />;\n",
    "Missing.screen.tsx": "export default () => <div />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Plain.screen.tsx", "<div");
  const inspect = await invoke({ operation: "inspect", source });
  assert.equal(inspect.shared, false);
  const apply = async (property, value) => invoke({ operation: "apply", source, scope: "component", property, value, expectedHash: inspect.hash });
  assert.match((await apply("width", "32px")).content, /w-\[32px\]/);
  assert.match((await apply("height", "18px")).content, /h-\[18px\]/);
  assert.match((await apply("borderRadius", "5px")).content, /rounded-\[5px\]/);
  assert.match((await apply("backgroundColor", "#111111")).content, /bg-\[#111111\]/);
  const color = await apply("color", "#222222");
  assert.match(color.content, /text-\[#222222\] text-center text-sm/);
  const weight = await apply("fontWeight", "500");
  assert.match(weight.content, /font-\[500\] font-\[Arial\]/);
  const missingSource = files.sourceAt("Missing.screen.tsx", "<div");
  const missing = await invoke({ operation: "inspect", source: missingSource });
  const added = await invoke({ operation: "apply", source: missingSource, property: "width", value: "calc(100% - 4px)", expectedHash: missing.hash });
  assert.match(added.content, /className="w-\[calc\(100%_-_4px\)\]"/);
});

test("updates selectionSource when a same-line callsite shifts its DOM definition", async (t) => {
  const files = await fixture({
    "Same.tsx": "export const Screen = () => <Card />; function Card({ className }) { return <div className={className}>x</div>; }\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const definition = files.sourceAt("Same.tsx", "<div");
  const callsite = files.sourceAt("Same.tsx", "<Card");
  const inspect = await invoke({ operation: "inspect", source: definition, instanceSource: callsite, scope: "instance" });
  const apply = await invoke({ operation: "apply", source: definition, instanceSource: callsite, scope: "instance", property: "padding", value: "4px", expectedHash: inspect.hash });
  assert.notEqual(apply.selectionSource, definition);
  assert.match(apply.content, /<Card className="p-\[4px\]" \/>.*<div className=\{className\}/);
});

test("keeps static expression literal quoting valid", async (t) => {
  const files = await fixture({
    "Expression.tsx": "export const Expression = () => <div className={'p-4'} />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Expression.tsx", "<div");
  const inspect = await invoke({ operation: "inspect", source, scope: "component" });
  const applied = await invoke({ operation: "apply", source, scope: "component", property: "padding", value: "8px", expectedHash: inspect.hash });
  assert.match(applied.content, /className=\{'p-\[8px\]'\}/);
});

test("replaces project named and typed arbitrary colors without touching variants or other text families", async (t) => {
  const files = await fixture({
    "Colors.tsx": "export const Colors = () => <div className=\"text-primary-bright text-textSecondary text-left text-[color:var(--brand)] text-[length:14px] text-[hsl(210_50%_40%)] hover:text-primary-bright bg-status-failed bg-[color:var(--surface)] bg-[url(http://example.test/a.png)] font-[Arial]\" />;\n",
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Colors.tsx", "<div");
  const inspect = await invoke({ operation: "inspect", source, scope: "component" });
  const color = await invoke({ operation: "apply", source, scope: "component", property: "color", value: "var(--next-text)", expectedHash: inspect.hash });
  assert.match(color.content, /text-\[color:var\(--next-text\)\] text-left text-\[length:14px\] hover:text-primary-bright/);
  assert.doesNotMatch(color.content, /text-primary-bright text-textSecondary/);
  const background = await invoke({ operation: "apply", source, scope: "component", property: "backgroundColor", value: "var(--next-surface)", expectedHash: inspect.hash });
  assert.match(background.content, /bg-\[color:var\(--next-surface\)\] bg-\[url\(http:\/\/example\.test\/a\.png\)\] font-\[Arial\]/);
  const fontSize = await invoke({ operation: "apply", source, scope: "component", property: "font-size", value: "calc(1rem + 2px)", expectedHash: inspect.hash });
  assert.match(fontSize.content, /text-\[color:var\(--brand\)\] text-\[length:calc\(1rem_\+_2px\)\] text-\[hsl\(210_50%_40%\)\]/);
});

test("replaces browser color functions and writes weight keywords as weight utilities", async (t) => {
  const files = await fixture({
    "Computed.tsx": 'export const View = () => <div className="text-[color(srgb_0.1_0.2_0.3)] bg-[color-mix(in_srgb,red_20%,blue)] font-medium font-[Arial] text-sm" />;\n',
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Computed.tsx", "<div");
  const inspected = await invoke({ operation: "inspect", source, scope: "component" });
  const color = await invoke({ operation: "apply", source, scope: "component", property: "color", value: "#123456", expectedHash: inspected.hash });
  assert.match(color.content, /text-\[#123456\]/); assert.doesNotMatch(color.content, /text-\[color\(srgb/);
  const background = await invoke({ operation: "apply", source, scope: "component", property: "background-color", value: "#abcdef", expectedHash: inspected.hash });
  assert.match(background.content, /bg-\[#abcdef\]/); assert.doesNotMatch(background.content, /bg-\[color-mix/);
  const weight = await invoke({ operation: "apply", source, scope: "component", property: "font-weight", value: "bold", expectedHash: inspected.hash });
  assert.match(weight.content, /font-bold font-\[Arial\]/);
  const variable = await invoke({ operation: "apply", source, scope: "component", property: "font-weight", value: "var(--weight)", expectedHash: inspected.hash });
  assert.match(variable.content, /font-\[number:var\(--weight\)\] font-\[Arial\]/);
  const inheritedSize = await invoke({ operation: "apply", source, scope: "component", property: "font-size", value: "inherit", expectedHash: inspected.hash });
  assert.match(inheritedSize.content, /text-\[length:inherit\]/);
});

test("fails closed when a component's runtime callsite marker is missing", async (t) => {
  const files = await fixture({
    "Shared.tsx": 'export default function Shared() { return <div className="p-4" />; }\n',
    "Nested.screen.tsx": 'export default function Screen() { return <Nested />; } function Nested() { return <div className="p-4"/>; }\n',
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  for (const file of ["Shared.tsx", "Nested.screen.tsx"]) {
    const source = files.sourceAt(file, "<div");
    const inspect = await invoke({ operation: "inspect", source, scope: "instance" });
    assert.equal(inspect.editable, false); assert.equal(inspect.reason, "instance_source_missing"); assert.equal(inspect.shared, true);
    const apply = await invoke({ operation: "apply", source, scope: "instance", property: "padding", value: "13px", expectedHash: inspect.hash });
    assert.equal(apply.reason, "instance_source_missing");
    const component = await invoke({ operation: "inspect", source, scope: "component" });
    assert.equal(component.editable, true); assert.equal(component.shared, true);
  }
});

test("does not apply a wrapper className to an unrelated selected inner element", async (t) => {
  const files = await fixture({
    "Card.tsx": 'export function Card({className}) { return <section className={className}><div className="p-4" /></section>; }\n',
    "Uses.tsx": 'import {Card} from "./Card"; export const Screen = () => <Card className="p-2"/>;\n',
  });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const instanceSource = files.sourceAt("Uses.tsx", "<Card");
  const outer = await invoke({ operation: "inspect", source: files.sourceAt("Card.tsx", "<section"), instanceSource, scope: "instance" });
  assert.equal(outer.editable, true);
  const innerSource = files.sourceAt("Card.tsx", "<div");
  const inner = await invoke({ operation: "inspect", source: innerSource, instanceSource, scope: "instance" });
  assert.equal(inner.editable, false); assert.equal(inner.reason, "no_classname_target");
  const rejected = await invoke({ operation: "apply", source: innerSource, instanceSource, scope: "instance", property: "padding", value: "13px", expectedHash: inner.hash });
  assert.equal(rejected.reason, "no_classname_target");
});

test("refuses a class edit that a known inline style would override", async (t) => {
  const files = await fixture({ "Inline.screen.tsx": 'export default () => <div className="p-4" style={{width:200}} />;\n' });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Inline.screen.tsx", "<div");
  const info = await invoke({ operation: "inspect", source, scope: "instance" });
  const width = await invoke({ operation: "apply", source, scope: "instance", property: "width", value: "13px", expectedHash: info.hash });
  assert.equal(width.reason, "inline_style_override");
  const padding = await invoke({ operation: "apply", source, scope: "instance", property: "padding", value: "13px", expectedHash: info.hash });
  assert.equal(padding.ok, true);
});

test("replaces slash-opacity colors and preserves the line-height size modifier", async (t) => {
  const files = await fixture({ "Modifiers.screen.tsx": 'export default () => <div className="bg-primary/10 text-white/60 text-sm/6" />;\n' });
  t.after(() => rm(files.directory, { recursive: true, force: true }));
  const source = files.sourceAt("Modifiers.screen.tsx", "<div");
  const info = await invoke({ operation: "inspect", source });
  const apply = (property, value) => invoke({ operation: "apply", source, property, value, expectedHash: info.hash });
  const background = await apply("background-color", "#112233");
  assert.match(background.content, /bg-\[#112233\] text-white\/60 text-sm\/6/);
  const color = await apply("color", "#223344");
  assert.match(color.content, /bg-primary\/10 text-\[#223344\] text-sm\/6/);
  const size = await apply("font-size", "13px");
  assert.match(size.content, /bg-primary\/10 text-white\/60 text-\[13px\] leading-6/);
});

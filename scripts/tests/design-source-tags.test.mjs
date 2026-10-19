import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import test from "node:test";
import { transformSync } from "@babel/core";
import { build } from "vite";
import designSourceTags from "../design-source-tags.mjs";

const root = process.cwd();

function transform(source, filename = resolve(root, "src/components/SourceFixture.tsx")) {
  return transformSync(source, {
    babelrc: false,
    configFile: false,
    filename,
    parserOpts: { plugins: ["jsx", "typescript"] },
    plugins: [[designSourceTags, { root }]],
  }).code;
}

test("adds true source metadata and class-name editability to intrinsic DOM nodes", () => {
  const output = transform(`const Card = () => (
  <main className="shell">
    <div className={"fixed"} />
    <button className={\`template\`} />
    <p className={ready ? "yes" : "no"} />
    <label className={getClassName()} />
    <span {...props} className="late" />
    <i />
    <article data-praxis-src="spoofed" data-praxis-class-kind="none" />
    <Widget className="custom" />
    <Forwarded {...props} />
    <UI.Badge />
  </main>
);`);

  assert.match(output, /<main className="shell" data-praxis-src="src\/components\/SourceFixture\.tsx:2:2" data-praxis-class-kind="static">/);
  assert.match(output, /<div className=\{"fixed"\} data-praxis-src="src\/components\/SourceFixture\.tsx:3:4" data-praxis-class-kind="static"/);
  assert.match(output, /<button className=\{`template`\} data-praxis-src="src\/components\/SourceFixture\.tsx:4:4" data-praxis-class-kind="dynamic"/);
  assert.match(output, /<p className=\{ready \? "yes" : "no"\} data-praxis-src="src\/components\/SourceFixture\.tsx:5:4" data-praxis-class-kind="dynamic"/);
  assert.match(output, /<label className=\{getClassName\(\)\} data-praxis-src="src\/components\/SourceFixture\.tsx:6:4" data-praxis-class-kind="dynamic"/);
  assert.match(output, /<span \{\.\.\.props\} className="late" data-praxis-src="src\/components\/SourceFixture\.tsx:7:4" data-praxis-class-kind="dynamic"/);
  assert.match(output, /<i data-praxis-src="src\/components\/SourceFixture\.tsx:8:4" data-praxis-class-kind="none"/);
  assert.match(output, /<article data-praxis-src="src\/components\/SourceFixture\.tsx:9:4" data-praxis-class-kind="none"/);
  assert.match(output, /<Widget className="custom" data-dojang-instance-src="src\/components\/SourceFixture\.tsx:10:4" data-dojang-instance-kind="static"/);
  assert.match(output, /<Forwarded \{\.\.\.props\} data-dojang-instance-src="src\/components\/SourceFixture\.tsx:11:4" data-dojang-instance-kind="dynamic"/);
  assert.match(output, /<UI\.Badge data-dojang-instance-src="src\/components\/SourceFixture\.tsx:12:4" data-dojang-instance-kind="none"/);
  assert.doesNotMatch(output, /<Widget[^>]*data-praxis-src/);
  assert.doesNotMatch(output, /spoofed/);
});

test("leaves files outside src and Design inspector chrome untagged", () => {
  const source = "export const view = <div className=\"x\" />;";
  assert.doesNotMatch(transform(source, resolve(root, "scripts/fixture.tsx")), /data-praxis-src/);
  assert.doesNotMatch(transform(source, resolve(root, "src/design/DesignCanvas.tsx")), /data-praxis-src/);
  assert.doesNotMatch(transform(source, "\0virtual:fixture"), /data-praxis-src/);
  assert.match(transform(source, resolve(root, "src/design/screens/fixture.screen.tsx")), /data-praxis-src="src\/design\/screens\/fixture\.screen\.tsx:1:20"/);
});

test("keeps static screen metadata referentially stable across development refreshes", () => {
  const filename = resolve(root, "src/design/screens/meta-cache.screen.tsx");
  const staticOutput = transform(`export const meta = {
  title: "Card",
  viewport: { width: 960, height: 640 },
  states: [null, true, -1]
};
export const Screen = () => <div />;`, filename);
  assert.match(staticOutput, /globalThis\.__dojangDesignMetaCache/);
  assert.match(staticOutput, /\.has\("src\/design\/screens\/meta-cache\.screen\.tsx\\0/);
  assert.match(staticOutput, /export const meta = _dojangMetaCache\.has/);

  const dynamicOutput = transform(`const title = getTitle();
export const meta = { title };
export const Screen = () => <div />;`, filename);
  assert.doesNotMatch(dynamicOutput, /__dojangDesignMetaCache/);
});

test("production Vite output has no source tags and excludes the design entry", async () => {
  const outputRoot = await mkdtemp(join(tmpdir(), "praxis-design-source-tags-"));
  try {
    const outDir = join(outputRoot, "dist");
    await build({
      configFile: resolve(root, "vite.config.ts"),
      logLevel: "silent",
      build: { outDir, emptyOutDir: true },
    });
    assert.equal(await readdir(outDir).then((files) => files.includes("design.html")), false);
    const assets = await readdir(join(outDir, "assets"));
    const scripts = await Promise.all(assets
      .filter((asset) => asset.endsWith(".js"))
      .map((asset) => readFile(join(outDir, "assets", asset), "utf8")));
    assert.ok(scripts.length > 0);
    assert.ok(scripts.every((script) => !script.includes("data-praxis-src")));
  } finally {
    await rm(outputRoot, { recursive: true, force: true });
  }
});

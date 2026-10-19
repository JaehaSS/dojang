// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { buildHtmlPreview, htmlAssetPath, type HtmlAssetReader } from "./html-preview-assets";

const PNG = "data:image/png;base64,iVBORw0KGgo=";
const SVG = "data:image/svg+xml;base64,PHN2Zy8+";
const reader = (files: Record<string, string>) => vi.fn<HtmlAssetReader>(async (path) => {
  if (!(path in files)) throw new Error("missing");
  return { kind: /\.(png|svg)$/.test(path) ? "image" : "text", content: files[path] };
});
const parse = (html: string) => new DOMParser().parseFromString(html, "text/html");

describe("HTML asset paths", () => {
  it("resolves stylesheet-relative paths, URL queries and encoded filenames", () => {
    expect(htmlAssetPath("../images/a%20b.png?v=1#icon", "docs/css/main.css")).toBe("docs/images/a b.png");
    expect(htmlAssetPath("./style.css", "/project/docs/page.html", "/project")).toBe("docs/style.css");
  });
  it.each(["../../outside.png", "%2e%2e/%2e%2e/secret.png", "/etc/a.png", "//host/a.png", "https://host/a.png", "file:///etc/a.png", "javascript:alert(1)", "~/.env", "%00.png", "%zz.png"])("rejects %s", (url) => {
    expect(htmlAssetPath(url, "docs/page.html")).toBeNull();
  });
  it("does not grant a sibling read to a document outside the project", () => {
    expect(htmlAssetPath("secret.png", "/private/page.html", "/project")).toBeNull();
  });
});

describe("static HTML bundling", () => {
  it("keeps document attributes/doctype, includes CSS and images, and preserves script denial", async () => {
    const read = reader({ "docs/style.css": "h1 { color: red; background: url('./logo.svg#mark') }", "docs/logo.svg": SVG });
    const input = '<!doctype html><html lang="ko"><head><link rel="stylesheet" href="style.css" media="screen"></head><body class="report"><h1>제목</h1><img src="logo.svg"><script>window.bad=1</script></body></html>';
    const result = await buildHtmlPreview(input, "docs/page.html", read);
    const doc = parse(result.html);
    expect(result.html).toMatch(/^<!DOCTYPE html>/i);
    expect(doc.documentElement.lang).toBe("ko");
    expect(doc.body.className).toBe("report");
    expect(doc.querySelector("link[rel=stylesheet]")).toBeNull();
    expect(doc.querySelector("style")?.media).toBe("screen");
    expect(doc.querySelector("style")?.textContent).toContain(`${SVG}#mark`);
    expect(doc.querySelector("img")?.getAttribute("src")).toBe(SVG);
    expect(doc.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute("content")).toContain("script-src 'none'");
    expect(result.unavailable).toEqual([]);
    expect(read.mock.calls.map(([path]) => path)).toEqual(["docs/style.css", "docs/logo.svg"]);
    expect(input).toContain('href="style.css"');
  });

  it("resolves nested imports and backgrounds against each stylesheet and keeps import conditions", async () => {
    const read = reader({
      "docs/css/main.css": '@import "nested/theme.css" layer(report) supports(display: grid) screen and (min-width: 300px); h1 { background:url(../logo.png) }',
      "docs/css/nested/theme.css": '.badge { background: url(../../logo.svg) }',
      "docs/logo.png": PNG, "docs/logo.svg": SVG,
    });
    const result = await buildHtmlPreview('<link rel="stylesheet" href="css/main.css"><h1>Title</h1>', "docs/page.html", read);
    const text = parse(result.html).querySelector("style")?.textContent ?? "";
    expect(text).toContain("@layer report");
    expect(text).toContain("@supports (display: grid)");
    expect(text).toContain("@media screen and (min-width: 300px)");
    expect(text).toContain(PNG);
    expect(text).toContain(SVG);
    expect(text).not.toContain("@import");
    expect(result.unavailable).toEqual([]);
  });

  it("includes inline CSS URLs, CSS escapes, srcset candidates, and inline SVG images", async () => {
    const read = reader({ "docs/a b.png": PNG, "docs/logo.svg": SVG });
    const result = await buildHtmlPreview('<style>.a{background: url(a\\20 b.png); content:"url(fake.png)"}</style><div style="background:url(logo.svg)"></div><picture><source srcset="logo.svg 2x"><img src="logo.svg" srcset="a%20b.png 1x, logo.svg 2x"></picture><svg><image href="logo.svg"/></svg>', "docs/page.html", read);
    const doc = parse(result.html);
    expect(doc.querySelector("style")?.textContent).toContain(PNG);
    expect(doc.querySelector("style")?.textContent).toContain('"url(fake.png)"');
    expect(doc.querySelector("div")?.getAttribute("style")).toContain(SVG);
    expect(doc.querySelector("img")?.getAttribute("srcset")).toBe(`${PNG} 1x, ${SVG} 2x`);
    expect(doc.querySelector("source")?.getAttribute("srcset")).toBe(`${SVG} 2x`);
    expect(doc.querySelector("image")?.getAttribute("href")).toBe(SVG);
    expect(result.unavailable).toEqual([]);
  });

  it("keeps data images and fragment references without file reads", async () => {
    const read = reader({});
    const result = await buildHtmlPreview(`<img src="${PNG}" srcset="${SVG} 1x, ${PNG} 2x"><style>.a{filter:url(#filter)}</style>`, "page.html", read);
    expect(parse(result.html).querySelector("img")?.getAttribute("srcset")).toBe(`${SVG} 1x, ${PNG} 2x`);
    expect(result.unavailable).toEqual([]);
    expect(read).not.toHaveBeenCalled();
  });

  it("reports missing/denied assets, blocks remote paths, and keeps the document visible", async () => {
    const read = reader({});
    const result = await buildHtmlPreview('<base href="https://host/"><link rel="stylesheet" href="missing.css"><img src="../private.png"><img src="https://host/a.png"><p>Visible</p>', "page.html", read);
    const doc = parse(result.html);
    expect(doc.querySelector("p")?.textContent).toBe("Visible");
    expect(doc.querySelector("base")).toBeNull();
    expect(result.unavailable).toEqual(["missing.css", "../private.png", "https://host/a.png"]);
    expect(read.mock.calls).toEqual([["missing.css"]]);
  });

  it("stops import cycles, ignores CSS-looking strings/comments, and disables alternate stylesheets", async () => {
    const read = reader({ "main.css": '@import "main.css"; /*url(no.png)*/ h1{content:"url(no.png)";color:red}' });
    const result = await buildHtmlPreview('<link rel="stylesheet" href="main.css"><link rel="alternate stylesheet" href="unused.css"><link rel="stylesheet" href="disabled.css" disabled>', "page.html", read);
    expect(result.unavailable).toEqual(["main.css"]);
    expect(read.mock.calls).toEqual([["main.css"]]);
    expect(parse(result.html).querySelector("style")?.textContent).toContain("color:red");
  });

  it("caps automatic resource reads and output expansion", async () => {
    const read = reader(Object.fromEntries(Array.from({ length: 70 }, (_, i) => [`${i}.png`, PNG])));
    const result = await buildHtmlPreview(Array.from({ length: 70 }, (_, i) => `<img src="${i}.png">`).join(""), "page.html", read);
    expect(read).toHaveBeenCalledTimes(64);
    expect(result.unavailable).toHaveLength(6);
    const large = reader({ "large.png": "data:image/png;base64," + "x".repeat(2 * 1024 * 1024) });
    const limited = await buildHtmlPreview('<img src="large.png">'.repeat(10), "page.html", large);
    expect(limited.html.length).toBeLessThan(8 * 1024 * 1024);
    expect(limited.unavailable).toEqual(["large.png"]);
    expect(large).toHaveBeenCalledTimes(1);
  });

  it("stops issuing asset reads after cancellation", async () => {
    const controller = new AbortController();
    const read = vi.fn<HtmlAssetReader>(async () => { controller.abort(); return { kind: "image", content: PNG }; });
    await expect(buildHtmlPreview('<img src="a.png"><img src="b.png">', "page.html", read, null, controller.signal)).rejects.toHaveProperty("name", "AbortError");
    expect(read).toHaveBeenCalledTimes(1);
  });
});

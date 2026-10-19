import postcss, { type AtRule, type ChildNode, type Declaration } from "postcss";
import valueParser from "postcss-value-parser";
import type { FileContent } from "./ipc";
import { relativeToWorktree } from "./agent-link";

export type HtmlAssetReader = (path: string) => Promise<Pick<FileContent, "kind" | "content">>;
export interface HtmlPreviewResult {
  html: string;
  unavailable: string[];
}

const MAX_ASSETS = 64;
const MAX_CONTENT = 8 * 1024 * 1024;
const MAX_IMPORT_DEPTH = 8;
const IMAGE_PATH = /\.(png|jpe?g|gif|webp|bmp|ico|svg)$/i;
// The document stays opaque and cannot request app/remote resources, even after a failed read.
const PREVIEW_CSP = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:; base-uri 'none'; form-action 'none'";

/** Resolve only project-relative resources. Outside-root read-only documents cannot resolve assets. */
export function htmlAssetPath(value: string, sourcePath: string, rootPath?: string | null): string | null {
  const raw = value.trim().split(/[?#]/, 1)[0];
  let link: string;
  try { link = decodeURIComponent(raw); } catch { return null; }
  if (!link || /^[a-z][a-z\d+.-]*:|^[\/\\~]|[\0-\x1f\x7f]/i.test(link)) return null;
  link = link.replace(/\\/g, "/");
  let source = sourcePath;
  if (source.startsWith("/") || /^[a-z]:[\\/]/i.test(source)) {
    source = rootPath ? relativeToWorktree(source, rootPath) ?? "" : "";
  }
  const parts = source.replace(/\\/g, "/").split("/");
  if (!source || parts.some((part) => !part || part === "." || part === "..")) return null;
  parts.pop();
  for (const part of link.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(part);
  }
  return parts.join("/") || null;
}

function cssUnescape(value: string): string {
  return value.replace(/\\(?:\r\n|[\r\n\f])/g, "").replace(/\\([\da-f]{1,6})[ \t\r\n\f]?|\\([^\r\n\f])/gi, (_, hex: string | undefined, char: string) => {
    if (!hex) return char;
    const code = parseInt(hex, 16);
    return String.fromCodePoint(code && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff) ? code : 0xfffd);
  });
}

function cssUrl(node: valueParser.Node): string | null {
  if (node.type === "string") return cssUnescape(node.value);
  if (node.type !== "function" || node.value.toLowerCase() !== "url") return null;
  const nodes = node.nodes.filter((item) => item.type !== "space" && item.type !== "comment");
  return nodes.length === 1 ? cssUnescape(nodes[0].value) : null;
}

/** Keep import media, supports and cascade-layer conditions when moving rules into an inline style. */
function importRules(css: string, nodes: valueParser.Node[]): ChildNode[] {
  const content = postcss.parse(css).nodes;
  const significant = nodes.filter((node) => node.type !== "space" && node.type !== "comment");
  let rules = content;
  let layer: string | null = null;
  let supports: string | null = null;
  let at = 0;
  const first = significant[at];
  if (first?.type === "word" && first.value.toLowerCase() === "layer") { layer = ""; at++; }
  else if (first?.type === "function" && first.value.toLowerCase() === "layer") {
    layer = valueParser.stringify(first.nodes); at++;
  }
  const next = significant[at];
  if (next?.type === "function" && next.value.toLowerCase() === "supports") {
    const condition = valueParser.stringify(next.nodes);
    supports = /^\s*\(/.test(condition) || /^\s*(not\b|selector\()/i.test(condition) ? condition : `(${condition})`;
    at++;
  }
  // Stringifying significant nodes alone would lose spaces between media-query words.
  const mediaStart = significant[at]?.sourceIndex;
  const mediaParams = mediaStart == null ? "" : valueParser.stringify(nodes.filter((node) => node.sourceIndex >= mediaStart)).trim();
  if (mediaParams) rules = [postcss.atRule({ name: "media", params: mediaParams }).append(rules)];
  if (supports != null) rules = [postcss.atRule({ name: "supports", params: supports }).append(rules)];
  if (layer != null) rules = [postcss.atRule({ name: "layer", params: layer }).append(rules)];
  return rules;
}

/** Bundle static resources through the existing bounded file reader; never expose a filesystem URL. */
export async function buildHtmlPreview(
  html: string,
  sourcePath: string,
  read: HtmlAssetReader,
  rootPath?: string | null,
  signal?: AbortSignal,
): Promise<HtmlPreviewResult> {
  const unavailable = new Set<string>();
  const cache = new Map<string, Promise<Pick<FileContent, "kind" | "content"> | null>>();
  let size = 0;
  let embeddedSize = html.length;
  const reserve = (length: number, path: string) => {
    if (embeddedSize + length > MAX_CONTENT) { unavailable.add(path); return false; }
    embeddedSize += length;
    return true;
  };
  const aborted = () => { if (signal?.aborted) throw new DOMException("Preview changed", "AbortError"); };
  const load = async (path: string) => {
    aborted();
    let pending = cache.get(path);
    if (!pending) {
      if (cache.size >= MAX_ASSETS) { unavailable.add(path); return null; }
      pending = read(path).then((file) => {
        aborted();
        size += file.content.length;
        if (size > MAX_CONTENT) { unavailable.add(path); return null; }
        return file;
      }).catch(() => {
        aborted();
        unavailable.add(path);
        return null;
      });
      cache.set(path, pending);
    }
    return pending;
  };
  const image = async (url: string, from: string): Promise<string> => {
    if (/^data:image\//i.test(url) || url.startsWith("#")) return url;
    const path = htmlAssetPath(url, from, rootPath);
    if (!path || !IMAGE_PATH.test(path)) { unavailable.add(url); return ""; }
    const file = await load(path);
    if (file?.kind !== "image" || !/^data:image\//i.test(file.content)) { unavailable.add(path); return ""; }
    if (!reserve(file.content.length, path)) return "";
    const hash = url.indexOf("#");
    return file.content + (hash < 0 ? "" : url.slice(hash));
  };
  const urls = async (value: string, from: string): Promise<string> => {
    const parsed = valueParser(value);
    const nodes: valueParser.FunctionNode[] = [];
    parsed.walk((node) => {
      if (node.type === "function" && node.value.toLowerCase() === "url") { nodes.push(node); return false; }
    });
    for (const node of nodes) {
      const url = cssUrl(node);
      if (url == null) continue;
      // Inline fonts/images remain usable; the preview CSP restricts which resource types can load.
      if (/^data:/i.test(url)) continue;
      const bundled = await image(url, from);
      node.nodes = [{ type: "string", quote: '"', value: bundled, sourceIndex: 0, sourceEndIndex: 0 }];
    }
    return parsed.toString();
  };
  const css = async (text: string, from: string, chain: string[]): Promise<string> => {
    aborted();
    let sheet;
    try { sheet = postcss.parse(text, { from }); }
    catch { unavailable.add(from); return ""; }
    const imports: AtRule[] = [];
    sheet.walkAtRules((rule) => {
      if (rule.name.toLowerCase() === "import") imports.push(rule);
      if (rule.name.toLowerCase() === "charset") rule.remove();
    });
    // Rewrite this sheet before inserting imported nodes, whose URLs use their own directory.
    const declarations: Declaration[] = [];
    sheet.walkDecls((decl) => { declarations.push(decl); });
    for (const decl of declarations) decl.value = await urls(decl.value, from);
    for (const rule of imports) {
      const params = valueParser(rule.params).nodes;
      const target = params.find((node) => node.type !== "space" && node.type !== "comment");
      const url = target ? cssUrl(target) : null;
      const path = url == null ? null : htmlAssetPath(url, from, rootPath);
      if (!path || !/\.css$/i.test(path) || chain.includes(path) || chain.length >= MAX_IMPORT_DEPTH) {
        unavailable.add(url ?? from); rule.remove(); continue;
      }
      const file = await load(path);
      if (file?.kind !== "text") { unavailable.add(path); rule.remove(); continue; }
      if (!reserve(file.content.length, path)) { rule.remove(); continue; }
      const imported = await css(file.content, path, [...chain, path]);
      rule.replaceWith(...importRules(imported, params.slice(params.indexOf(target!) + 1)));
    }
    return sheet.toString();
  };

  // A detached document is never mounted in the app; only its serialized text enters the sandbox.
  const doc = new DOMParser().parseFromString(html, "text/html");
  const policy = doc.createElement("meta");
  policy.httpEquiv = "Content-Security-Policy";
  policy.content = PREVIEW_CSP;
  doc.head.prepend(policy);
  doc.querySelectorAll("base").forEach((base) => base.remove());
  for (const link of doc.querySelectorAll<HTMLLinkElement>('link[rel~="stylesheet" i][href]')) {
    const url = link.getAttribute("href")!;
    if (link.hasAttribute("disabled") || link.relList.contains("alternate")) { link.remove(); continue; }
    const path = htmlAssetPath(url, sourcePath, rootPath);
    const file = path && /\.css$/i.test(path) ? await load(path) : null;
    if (file?.kind !== "text") { unavailable.add(url); link.remove(); continue; }
    if (!reserve(file.content.length, path!)) { link.remove(); continue; }
    const style = doc.createElement("style");
    if (link.media) style.media = link.media;
    style.textContent = (await css(file.content, path!, [path!])).replace(/<\/style/gi, "<\\/style");
    link.replaceWith(style);
  }
  for (const style of doc.querySelectorAll("style")) {
    // Already-bundled links contain only data URLs and no imports, so this is harmless.
    style.textContent = (await css(style.textContent ?? "", sourcePath, [])).replace(/<\/style/gi, "<\\/style");
  }
  for (const element of doc.querySelectorAll<HTMLElement>("[style]")) {
    element.setAttribute("style", await urls(element.getAttribute("style")!, sourcePath));
  }
  for (const img of doc.querySelectorAll<HTMLImageElement>("img[src], input[type=image][src]")) {
    img.setAttribute("src", await image(img.getAttribute("src")!, sourcePath));
  }
  // Preserve descriptors and data-URL commas rather than splitting a srcset on every comma.
  for (const element of doc.querySelectorAll("img[srcset], source[srcset]")) {
    const srcset = element.getAttribute("srcset")!;
    const candidates: string[] = [];
    let at = 0;
    while (at < srcset.length) {
      while (/[\s,]/.test(srcset[at] ?? "") && at < srcset.length) at++;
      const start = at;
      while (at < srcset.length && !/\s/.test(srcset[at])) at++;
      const token = srcset.slice(start, at);
      const url = token.replace(/,+$/, "");
      const descriptorStart = at;
      if (!token.endsWith(",")) {
        let depth = 0;
        while (at < srcset.length) {
          if (srcset[at] === "(") depth++;
          if (srcset[at] === ")") depth--;
          if (srcset[at] === "," && depth === 0) break;
          at++;
        }
      }
      const descriptor = srcset.slice(descriptorStart, at).trim();
      if (srcset[at] === ",") at++;
      const bundled = url ? await image(url, sourcePath) : "";
      if (bundled) candidates.push(bundled + (descriptor ? ` ${descriptor}` : ""));
    }
    if (candidates.length) element.setAttribute("srcset", candidates.join(", "));
    else element.removeAttribute("srcset");
  }
  for (const element of doc.querySelectorAll("image[href], image[xlink\\:href]")) {
    for (const attr of ["href", "xlink:href"]) {
      const url = element.getAttribute(attr);
      if (url != null) element.setAttribute(attr, await image(url, sourcePath));
    }
  }
  aborted();
  const doctype = doc.doctype ? new XMLSerializer().serializeToString(doc.doctype) : "";
  return { html: doctype + doc.documentElement.outerHTML, unavailable: [...unavailable] };
}

// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FileContent } from "../../lib/ipc";
import { MarkdownDoc } from "./MarkdownDoc";

vi.mock("./Mermaid", () => ({ Mermaid: () => null }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const image = (value = "cG5n"): FileContent => ({ kind: "image", content: `data:image/png;base64,${value}`, mtime: 1 });
const render = async (props: Partial<Parameters<typeof MarkdownDoc>[0]> = {}) => {
  await act(async () => root.render(<MarkdownDoc
    text="![도표](images/chart.png)" dark documentPath="docs/note.md" rootPath="/workspace" {...props}
  />));
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("Markdown document images", () => {
  it("loads reference-style images from the document directory and preserves alt, title and source line", async () => {
    const readFile = vi.fn(async () => image());
    await render({ text: '![도표][chart]\n\n[chart]: ../assets/chart.png "원본 도표"', readFile });
    expect(readFile).toHaveBeenCalledWith("assets/chart.png");
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe(image().content);
    expect(img?.alt).toBe("도표");
    expect(img?.title).toBe("원본 도표");
    expect(img?.getAttribute("data-md-line")).toBe("1");
  });

  it("reads file URIs while keeping the sanitizer on anchor links", async () => {
    const readFile = vi.fn(async () => image());
    await render({ text: "![도표](file:///workspace/assets/chart.png)\n\n[unsafe](javascript:alert)\n\n[file](file:///workspace/note.md)", readFile });
    expect(readFile).toHaveBeenCalledWith("assets/chart.png");
    expect(container.querySelector("img")?.getAttribute("src")).toBe(image().content);
    expect([...container.querySelectorAll("a")].map((a) => a.getAttribute("href"))).toEqual(["", ""]);
  });

  it("shows embedded data images without trying to read a file", async () => {
    const readFile = vi.fn();
    await render({ text: `![도표](${image().content})`, readFile });
    expect(container.querySelector("img")?.getAttribute("src")).toBe(image().content);
    expect(readFile).not.toHaveBeenCalled();
  });

  it("does not read local or remote images when the caller disables image loading", async () => {
    const readFile = vi.fn(async () => image());
    await render({ text: "![로컬](images/chart.png)\n\n![원격](https://example.test/chart.png)", readFile, loadImages: false });
    expect(readFile).not.toHaveBeenCalled();
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("[이미지: 로컬]");
    expect(container.textContent).toContain("[이미지: 원격]");
  });

  it.each([
    { kind: "too_large", content: "", mtime: 1 },
    { kind: "text", content: "not an image", mtime: 1 },
    { kind: "image", content: "https://example.test/image.png", mtime: 1 },
  ] satisfies FileContent[])("shows a readable fallback for unsupported reader result $kind", async (file) => {
    await render({ readFile: async () => file });
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("이미지를 불러오지 못했습니다: 도표");
  });

  it("shows a readable failure without handing missing local paths to the browser", async () => {
    await render({ readFile: async () => { throw new Error("파일을 찾을 수 없습니다"); } });
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("span")?.title).toContain("파일을 찾을 수 없습니다");
  });

  it("does not read workspace escapes or executable URL schemes", async () => {
    const readFile = vi.fn(async () => image());
    await render({ text: "![escape](../../private.png)\n\n![unsafe](javascript:alert)", readFile });
    expect(readFile).not.toHaveBeenCalled();
    expect(container.querySelector("img")).toBeNull();
  });

  it("does not display a late result after switching the source workspace", async () => {
    let finish!: (file: FileContent) => void;
    const first = vi.fn(() => new Promise<FileContent>((resolve) => { finish = resolve; }));
    const second = vi.fn(async () => image("bmV3"));
    await render({ readFile: first });
    expect(container.querySelector("img")).toBeNull();
    await render({ readFile: second });
    expect(container.querySelector("img")?.getAttribute("src")).toBe(image("bmV3").content);
    await act(async () => finish(image("b2xk")));
    expect(container.querySelector("img")?.getAttribute("src")).toBe(image("bmV3").content);
  });

  it("resolves the same image spelling again when switching document directories", async () => {
    const readFile = vi.fn(async (path: string) => image(path === "docs/images/chart.png" ? "b25l" : "dHdv"));
    await render({ readFile });
    await render({ readFile, documentPath: "notes/note.md" });
    expect(readFile.mock.calls).toEqual([["docs/images/chart.png"], ["notes/images/chart.png"]]);
    expect(container.querySelector("img")?.getAttribute("src")).toBe(image("dHdv").content);
  });
});

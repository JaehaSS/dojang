// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FileContent } from "../../lib/ipc";
import { Markdown } from "./Markdown";
import { SessionImageScope } from "./SessionImageContext";
import type { DocumentImageReader } from "./MarkdownImage";

vi.mock("./Mermaid", () => ({ Mermaid: () => null }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let container: HTMLDivElement;
let root: Root;
const image: FileContent = { kind: "image", content: "data:image/png;base64,cG5n", mtime: 1 };
const render = async (text: string, readFile?: DocumentImageReader, rootPath = "/workspace") => {
  await act(async () => root.render(<SessionImageScope rootPath={rootPath} readFile={readFile}>
    <Markdown text={text} />
  </SessionImageScope>));
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

describe("Session message images", () => {
  it.each(["assets/chart.png", "/workspace/assets/chart.png", "file:///workspace/assets/chart.png"])("loads %s through the session reader", async (path) => {
    const readFile = vi.fn(async () => image);
    await render(`![도표](${path} "분석 결과")`, readFile);
    expect(readFile).toHaveBeenCalledWith("assets/chart.png");
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe(image.content);
    expect(img?.alt).toBe("도표");
    expect(img?.title).toBe("분석 결과");
    expect(img?.className).toContain("max-h-80");
  });
  it("passes an emitted absolute output path outside the workspace to the guarded reader", async () => {
    const readFile = vi.fn(async () => image);
    await render("![생성 결과](</Users/owner/generated image.png>)", readFile);
    expect(readFile).toHaveBeenCalledWith("/Users/owner/generated image.png");
    expect(container.querySelector("img")?.getAttribute("src")).toBe(image.content);
  });
  it("shows embedded and remote images without reading local files", async () => {
    const readFile = vi.fn();
    await render(`![결과](${image.content})\n\n![웹](https://example.test/chart.png)`, readFile);
    expect([...container.querySelectorAll("img")].map(img => img.getAttribute("src"))).toEqual([image.content, "https://example.test/chart.png"]);
    expect(readFile).not.toHaveBeenCalled();
  });
  it.each([
    async () => { throw new Error("파일을 찾을 수 없습니다"); },
    async (): Promise<FileContent> => ({ kind: "too_large", content: "", mtime: 1 }),
  ])("shows a readable failure for unavailable files", async (readFile) => {
    await render("![도표](chart.png)", readFile);
    expect(container.querySelector("img")).toBeNull();
    expect(container.textContent).toContain("이미지를 불러오지 못했습니다: 도표");
  });
  it("does not read executable schemes or workspace escapes", async () => {
    const readFile = vi.fn(async () => image);
    await render("![unsafe](javascript:alert)\n\n![escape](../private.png)", readFile);
    expect(readFile).not.toHaveBeenCalled();
    expect(container.querySelector("img")).toBeNull();
  });
  it("discards a pending image when switching sessions", async () => {
    let finish!: (file: FileContent) => void;
    const oldReader = () => new Promise<FileContent>(resolve => { finish = resolve; });
    await render("![도표](chart.png)", oldReader);
    const newImage = { ...image, content: "data:image/png;base64,bmV3" };
    await render("![도표](chart.png)", async () => newImage, "/other");
    await act(async () => finish(image));
    expect(container.querySelector("img")?.getAttribute("src")).toBe(newImage.content);
  });
});

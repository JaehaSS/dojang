// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";
import type { FileContent } from "./ipc";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), fsRead: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("./ipc", () => ({ fsRead: mocks.fsRead }));

import {
  prepareArtifactSave,
  artifactReviewContext,
  reviewArtifactGet,
  reviewArtifactList,
  reviewArtifactSave,
} from "./review-artifacts";

const text = (content: string): FileContent => ({ kind: "text", content, mtime: 1 });
const image = (content = "data:image/png;base64,cG5n"): FileContent => ({ kind: "image", content, mtime: 1 });

describe("review artifacts", () => {
  it("prepares an HTML snapshot with frozen resource contents and CSP", async () => {
    mocks.fsRead.mockImplementation(async (_ref: unknown, path: string) => path === "report.html"
      ? text('<link rel="stylesheet" href="assets/page.css"><img src="assets/chart.png">')
      : path === "assets/page.css" ? text(".chart { background: url(chart.png) }")
        : image());

    const prepared = await prepareArtifactSave({ taskId: 7, sourceId: "source", resultRevision: 4, relativePath: "report.html" });

    expect(prepared.kind).toBe("html");
    expect(prepared.preview_html).toContain("Content-Security-Policy");
    expect(prepared.preview_html).toContain("data:image/png;base64,cG5n");
    expect(prepared.resources).toEqual([
      { relative_path: "assets/page.css", expected_content: ".chart { background: url(chart.png) }" },
      { relative_path: "assets/chart.png", expected_content: "data:image/png;base64,cG5n" },
    ]);
  });

  it("renders Markdown before freezing local image data in a standalone document", async () => {
    mocks.fsRead.mockImplementation(async (_ref: unknown, path: string) => path === "notes/readme.md"
      ? text("# 제목\n\n![차트](../images/chart.png)") : image());

    const prepared = await prepareArtifactSave({ taskId: 7, sourceId: "source", resultRevision: 4, relativePath: "notes/readme.md" });

    expect(prepared.kind).toBe("markdown");
    expect(prepared.preview_html).toContain("<h1>제목</h1>");
    expect(prepared.preview_html).toContain("data:image/png;base64,cG5n");
    expect(prepared.resources).toEqual([{ relative_path: "images/chart.png", expected_content: "data:image/png;base64,cG5n" }]);
  });

  it("keeps an image as its saved data URL and rejects unsupported source kinds", async () => {
    mocks.fsRead.mockResolvedValueOnce(image());
    await expect(prepareArtifactSave({ taskId: 7, sourceId: "source", resultRevision: 4, relativePath: "chart.png" }))
      .resolves.toMatchObject({ kind: "image", preview_html: null, expected_content: "data:image/png;base64,cG5n" });
    mocks.fsRead.mockResolvedValueOnce({ kind: "binary", content: "", mtime: 1 } satisfies FileContent);
    await expect(prepareArtifactSave({ taskId: 7, sourceId: "source", resultRevision: 4, relativePath: "archive.zip" }))
      .rejects.toThrow("HTML, Markdown, 이미지 파일만 저장할 수 있습니다");
  });

  it("uses the canonical immutable-artifact IPC commands", async () => {
    mocks.invoke.mockResolvedValue({});
    await reviewArtifactList(1);
    await reviewArtifactGet(1, 9);
    await reviewArtifactSave(1, {
      source_id: "s", expected_revision: 2, relative_path: "report.html", expected_content: "<p>x</p>", preview_html: "<html></html>", unavailable: [], resources: [],
    });
    expect(mocks.invoke.mock.calls.map(([command]) => command)).toEqual([
      "review_artifact_list", "review_artifact_get", "review_artifact_save",
    ]);
    expect(mocks.invoke.mock.calls[1][1]).toEqual({ id: 1, artifactId: 9 });
  });

  it("limits Korean snapshot excerpts by UTF-8 bytes while retaining version references", () => {
    const previous = { id: 1, task_id: 1, source_id: "s", result_revision: 2, relative_path: "before.md", version: 1, kind: "markdown" as const, content_hash: "before-hash", created_at: 1, base_revision: null, unavailable: [], content: "한".repeat(2_000), preview_html: "<html></html>" };
    const next = { ...previous, id: 2, relative_path: "after.md", content_hash: "after-hash" };
    const context = artifactReviewContext(previous, next);
    expect(context).toContain("저장 버전 #1 (s; before.md)");
    expect(context).toContain("해시: after-hash; 결과 revision: 2");
    expect(context).toContain("[저장된 본문 일부 생략]");
    expect(new TextEncoder().encode(context).length).toBeLessThan(2_700);
  });

  it("clips large non-BMP text without splitting Unicode characters", () => {
    const saved = { id: 1, task_id: 1, source_id: "s", result_revision: 1, relative_path: "emoji.md", version: 1, kind: "markdown" as const, content_hash: "hash", created_at: 1, base_revision: null, unavailable: [], content: "🙂".repeat(100_000), preview_html: "<html></html>" };
    const context = artifactReviewContext(saved, saved);
    expect(context).not.toContain("\ufffd");
    expect(context).toContain("[저장된 본문 일부 생략]");
    expect(new TextEncoder().encode(context).length).toBeLessThan(2_700);
  });
});

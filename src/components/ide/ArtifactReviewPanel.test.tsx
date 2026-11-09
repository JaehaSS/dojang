// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ArtifactReviewPanel } from "./ArtifactReviewPanel";
import type { ArtifactSnapshot, ArtifactVersion } from "../../lib/review-artifacts";

const mocks = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn(), save: vi.fn(), prepare: vi.fn(), context: vi.fn() }));
vi.mock("../../lib/review-artifacts", () => ({
  reviewArtifactList: mocks.list,
  reviewArtifactGet: mocks.get,
  reviewArtifactSave: mocks.save,
  prepareArtifactSave: mocks.prepare,
  artifactReviewContext: mocks.context,
}));
vi.mock("./HtmlDoc", () => ({ HtmlDoc: ({ title, html }: { title: string; html: string }) => <iframe title={title} srcDoc={html} /> }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;
const version = (id: number, path = "report.html"): ArtifactVersion => ({
  id, task_id: 1, source_id: "source", result_revision: 3, relative_path: path, version: id, kind: "html", content_hash: `hash-${id}`, created_at: id, base_revision: null, unavailable: [],
});
const snapshot = (id: number, content = `saved ${id}`): ArtifactSnapshot => ({ ...version(id), content, preview_html: `<html><body>${content}</body></html>` });
const render = async (props: Partial<React.ComponentProps<typeof ArtifactReviewPanel>> = {}) => {
  await act(async () => root.render(<ArtifactReviewPanel taskId={1} sourceId="source" resultRevision={3} paths={["report.html", "notes/readme.md"]} onAttachReview={() => {}} onOpenFile={() => {}} {...props} />));
};
const select = (label: string) => host.querySelector(`select[aria-label="${label}"]`) as HTMLSelectElement;
const button = (text: string) => [...host.querySelectorAll("button")].find((node) => node.textContent === text)!;

beforeEach(() => {
  vi.clearAllMocks();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  mocks.list.mockResolvedValue([version(2), version(1)]);
  mocks.get.mockImplementation(async (_taskId: number, artifactId: number) => snapshot(artifactId));
  mocks.context.mockReturnValue("immutable version references");
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

describe("ArtifactReviewPanel", () => {
  it("loads two saved versions into accessible before/after selectors and frozen preview frames", async () => {
    await render();
    expect(select("이전 버전").value).toBe("1");
    expect(select("이후 버전").value).toBe("2");
    expect(host.querySelector('iframe[title="이전 버전: report.html"]')?.getAttribute("srcdoc")).toContain("saved 1");
    expect(host.querySelector('iframe[title="이후 버전: report.html"]')?.getAttribute("srcdoc")).toContain("saved 2");
  });

  it("does not let a late snapshot overwrite the newly selected side", async () => {
    let resolveOld!: (value: ArtifactSnapshot) => void;
    let resolveNew!: (value: ArtifactSnapshot) => void;
    mocks.get.mockImplementation((_taskId: number, artifactId: number) => new Promise<ArtifactSnapshot>((resolve) => {
      if (artifactId === 1) resolveOld = resolve;
      else resolveNew = resolve;
    }));
    await render();
    await act(async () => {
      select("이전 버전").value = "2";
      select("이전 버전").dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(select("이전 버전").value).toBe("2");
    expect(button("비교 대상을 검토 사유에 연결").disabled).toBe(true);
    await act(async () => resolveOld(snapshot(1, "old response")));
    expect(button("비교 대상을 검토 사유에 연결").disabled).toBe(true);
    await act(async () => resolveNew(snapshot(2)));
    expect(host.querySelector('iframe[title="이전 버전: report.html"]')?.getAttribute("srcdoc")).toContain("saved 2");
    expect(host.innerHTML).not.toContain("old response");
  });

  it("keeps a loaded preview when the same option is selected or the result revision refreshes", async () => {
    await render();
    const preview = () => host.querySelector('iframe[title="이전 버전: report.html"]')?.getAttribute("srcdoc");
    expect(preview()).toContain("saved 1");
    await act(async () => {
      select("이전 버전").value = "1";
      select("이전 버전").dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(preview()).toContain("saved 1");
    await render({ resultRevision: 4 });
    expect(preview()).toContain("saved 1");
    expect(mocks.list).toHaveBeenCalledTimes(2);
  });

  it("connects both immutable references to the review reason without sending a request", async () => {
    const attach = vi.fn();
    await render({ onAttachReview: attach });
    await act(async () => button("비교 대상을 검토 사유에 연결").click());
    expect(attach).toHaveBeenCalledOnce();
    expect(mocks.context).toHaveBeenCalledWith(expect.objectContaining({ id: 1, content_hash: "hash-1" }), expect.objectContaining({ id: 2, content_hash: "hash-2" }));
    expect(attach).toHaveBeenCalledWith("immutable version references");
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it("only reads current source when the explicit save action is pressed", async () => {
    mocks.prepare.mockResolvedValue({ source_id: "source", expected_revision: 3, relative_path: "report.html", expected_content: "x", preview_html: "<html></html>", unavailable: [], resources: [] });
    mocks.save.mockResolvedValue(snapshot(3));
    const open = vi.fn();
    await render({ onOpenFile: open });
    expect(mocks.prepare).not.toHaveBeenCalled();
    await act(async () => button("현재 원본 열기").click());
    expect(open).toHaveBeenCalledWith("report.html");
    await act(async () => button("검토 버전 저장").click());
    expect(mocks.prepare).toHaveBeenCalledWith({ taskId: 1, sourceId: "source", resultRevision: 3, relativePath: "report.html" });
    expect(mocks.save).toHaveBeenCalledOnce();
  });

  it("does not let a list started before save erase the newly saved version", async () => {
    let resolveList!: (versions: ArtifactVersion[]) => void;
    mocks.list.mockReturnValueOnce(new Promise<ArtifactVersion[]>((resolve) => { resolveList = resolve; }));
    mocks.prepare.mockResolvedValue({ source_id: "source", expected_revision: 3, relative_path: "report.html", expected_content: "x", preview_html: "<html></html>", unavailable: [], resources: [] });
    mocks.save.mockResolvedValue(snapshot(3));
    await render();
    await act(async () => button("검토 버전 저장").click());
    await act(async () => resolveList([]));
    expect([...select("이후 버전").options].map((option) => option.value)).toContain("3");
    expect(select("이후 버전").disabled).toBe(false);
  });
});

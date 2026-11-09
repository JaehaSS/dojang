// @vitest-environment jsdom
import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { SpecEvidencePanel } from "./SpecEvidencePanel";
import { taskSpecGet, taskSpecList, taskSpecRead, taskSpecSave, taskSpecSearch, taskSpecValidateLinks, type SpecLinkReport } from "../../lib/task-specs";

vi.mock("../../lib/task-specs", async (original) => ({ ...(await original<typeof import("../../lib/task-specs")>()), taskSpecSearch: vi.fn(), taskSpecRead: vi.fn(), taskSpecSave: vi.fn(), taskSpecList: vi.fn(), taskSpecGet: vi.fn(), taskSpecValidateLinks: vi.fn() }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const hash = "a".repeat(64), otherHash = "b".repeat(64);
const hit = { relative_path: "docs/auth.md", title: "인증 명세", snippet: "로그인 정책", line: 7 };
const liveDocument = { relative_path: hit.relative_path, title: hit.title, content_hash: hash, byte_length: 11, content: "first source" };
const snapshot = { ...liveDocument, id: 8, task_id: 1, created_at: 1, base_revision: null };
const linkReport: SpecLinkReport = { relative_path: hit.relative_path, content_hash: hash, checked_at: 1, truncated: false, summary: { total: 2, ok: 1, missing: 0, outside_scope: 0, unsupported: 0, anchor_missing: 0, unreadable: 0, external: 1 }, links: [{ source_line: 3, target: "guide.md", state: "ok", resolved_path: "docs/guide.md" }, { source_line: 4, target: "https://example.test", state: "external", resolved_path: null }] };
let root: Root, host: HTMLDivElement;
const button = (label: string) => [...host.querySelectorAll("button")].find((element) => element.textContent === label || element.textContent?.includes(label)) as HTMLButtonElement;
const click = async (element: HTMLButtonElement | undefined) => {
  expect(element).toBeTruthy();
  await act(async () => {
    element?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();
  });
};
const input = () => host.querySelector('[aria-label="명세 검색어"]') as HTMLInputElement;
const setQuery = async (value: string) => {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input(), value);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
};
const deferred = <T,>() => { let resolve!: (value: T) => void; let reject!: (error: unknown) => void; const promise = new Promise<T>((ok, fail) => { resolve = ok; reject = fail; }); return { promise, resolve, reject }; };

beforeEach(() => {
  vi.clearAllMocks();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  vi.mocked(taskSpecList).mockResolvedValue([]);
  vi.mocked(taskSpecSearch).mockResolvedValue({ hits: [hit], truncated: false, scanned_files: 4, skipped_files: 2 });
  vi.mocked(taskSpecRead).mockResolvedValue(liveDocument);
  vi.mocked(taskSpecSave).mockResolvedValue(snapshot);
  vi.mocked(taskSpecGet).mockResolvedValue(snapshot);
  vi.mocked(taskSpecValidateLinks).mockResolvedValue(linkReport);
});
afterEach(() => { act(() => root.unmount()); host.remove(); });

async function render(taskId = 1, props: Partial<ComponentProps<typeof SpecEvidencePanel>> = {}) {
  await act(async () => root.render(<SpecEvidencePanel taskId={taskId} onOpenFile={vi.fn()} onAttachReview={vi.fn()} onAttachInput={vi.fn()} {...props} />));
}
async function searchAndRead() {
  await setQuery("인증");
  expect(input().value).toBe("인증");
  expect(button("명세 검색").disabled).toBe(false);
  await click(button("명세 검색"));
  expect(taskSpecSearch).toHaveBeenCalled();
  expect(host.textContent).toContain("인증 명세");
  await click(button("인증 명세"));
}

it("searches explicit keyword candidates, reads the live source, and saves its hash", async () => {
  vi.mocked(taskSpecList).mockResolvedValueOnce([]).mockResolvedValueOnce([snapshot]);
  await render();
  expect(button("명세 검색").disabled).toBe(true);
  await searchAndRead();
  expect(taskSpecSearch).toHaveBeenCalledWith(1, "인증");
  expect(host.textContent).toContain("키워드 일치 후보");
  expect(host.textContent).toContain("탐색 4개 · 건너뜀 2개");
  expect(host.textContent).toContain("first source");
  await click(button("명세 버전 보존"));
  expect(taskSpecSave).toHaveBeenCalledWith(1, { relative_path: "docs/auth.md", expected_hash: hash });
  expect(host.textContent).toContain("보존 원문");
  expect(host.textContent).toContain("first source");
});

it("keeps separately captured bodies when the same source changes", async () => {
  const secondDocument = { ...liveDocument, content_hash: otherHash, content: "changed source" };
  const secondSnapshot = { ...snapshot, id: 9, content_hash: otherHash, content: "changed source" };
  vi.mocked(taskSpecList).mockResolvedValueOnce([]).mockResolvedValueOnce([snapshot]).mockResolvedValueOnce([secondSnapshot, snapshot]);
  vi.mocked(taskSpecRead).mockResolvedValueOnce(liveDocument).mockResolvedValueOnce(secondDocument);
  vi.mocked(taskSpecSave).mockResolvedValueOnce(snapshot).mockResolvedValueOnce(secondSnapshot);
  await render(); await searchAndRead();
  await click(button("명세 버전 보존"));
  await click(button("명세 검색"));
  await click(button("인증 명세"));
  await click(button("명세 버전 보존"));
  expect(host.textContent).toContain("changed source");
  // The saved first record can be revisited even though the live source has since changed.
  const firstVersion = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="보존 명세"] button')].find((element) => element.textContent?.includes("#8"));
  await click(firstVersion);
  expect(taskSpecGet).toHaveBeenCalledWith(1, 8);
});

it("clears invalidated current and frozen loading indicators", async () => {
  const slowRead = deferred<typeof liveDocument>(), slowSnapshot = deferred<typeof snapshot>();
  vi.mocked(taskSpecList).mockResolvedValue([snapshot]);
  vi.mocked(taskSpecRead).mockReturnValueOnce(slowRead.promise);
  vi.mocked(taskSpecGet).mockReturnValueOnce(slowSnapshot.promise);
  await render();
  const version = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="보존 명세"] button')].find((element) => element.textContent?.includes("#8"));
  await click(version);
  expect(host.textContent).toContain("보존 원문을 불러오는 중입니다.");
  await setQuery("인증");
  await click(button("명세 검색"));
  expect(host.textContent).not.toContain("보존 원문을 불러오는 중입니다.");
  await click(button("인증 명세"));
  expect(host.textContent).toContain("현재 문서를 불러오는 중입니다.");
  const savedVersion = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="보존 명세"] button')].find((element) => element.textContent?.includes("#8"));
  await click(savedVersion);
  expect(host.textContent).not.toContain("현재 문서를 불러오는 중입니다.");
  await act(async () => { slowRead.resolve(liveDocument); slowSnapshot.resolve(snapshot); });
});

it("shows frozen content after a reload and attaches a reference without sending", async () => {
  vi.mocked(taskSpecList).mockResolvedValue([snapshot]);
  const inputAttach = vi.fn(), reviewAttach = vi.fn();
  await render(1, { onAttachInput: inputAttach, onAttachReview: reviewAttach });
  const version = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="보존 명세"] button')].find((element) => element.textContent?.includes("#8"));
  await click(version);
  expect(host.textContent).toContain("first source");
  await click(button("작업 입력에 연결"));
  await click(button("검토 사유에 연결"));
  expect(inputAttach).toHaveBeenCalledWith(expect.stringContaining(`[[dojang-spec:8:${hash}]]`));
  expect(reviewAttach).toHaveBeenCalledWith(expect.stringContaining("명세 근거: docs/auth.md"));
  expect(taskSpecSearch).not.toHaveBeenCalled();
});

it("does not apply stale search or read results after a task changes", async () => {
  const oldSearch = deferred<{ hits: typeof hit[]; truncated: boolean; scanned_files: number; skipped_files: number }>();
  vi.mocked(taskSpecSearch).mockReturnValueOnce(oldSearch.promise);
  await render();
  await setQuery("인증");
  await click(button("명세 검색"));
  await render(2);
  await act(async () => oldSearch.resolve({ hits: [hit], truncated: false, scanned_files: 1, skipped_files: 0 }));
  expect(host.textContent).not.toContain("인증 명세");

  const oldRead = deferred<typeof liveDocument>();
  vi.mocked(taskSpecSearch).mockResolvedValueOnce({ hits: [hit], truncated: false, scanned_files: 1, skipped_files: 0 });
  vi.mocked(taskSpecRead).mockReturnValueOnce(oldRead.promise);
  await setQuery("인증");
  await click(button("명세 검색"));
  await click(button("인증 명세"));
  await render(3);
  await act(async () => oldRead.resolve(liveDocument));
  expect(host.textContent).not.toContain("first source");
});

it("ignores a save completion after unmount and keeps mutations disabled", async () => {
  const pendingSave = deferred<typeof snapshot>();
  vi.mocked(taskSpecSave).mockReturnValueOnce(pendingSave.promise);
  await render(); await searchAndRead();
  await click(button("명세 버전 보존"));
  act(() => root.unmount());
  await act(async () => pendingSave.resolve(snapshot));

  root = createRoot(host);
  await render(1, { disabled: true }); await searchAndRead();
  expect(button("명세 버전 보존").disabled).toBe(true);
});

it("keeps search, save, and list errors retryable separately and shows truncation", async () => {
  vi.mocked(taskSpecList).mockRejectedValueOnce(new Error("목록 실패")).mockResolvedValue([]);
  vi.mocked(taskSpecSearch).mockRejectedValueOnce(new Error("검색 실패")).mockResolvedValue({ hits: [], truncated: true, scanned_files: 40, skipped_files: 3 });
  await render();
  expect(host.textContent).toContain("보존 명세 목록 조회 실패: 목록 실패");
  await setQuery("인증");
  await click(button("명세 검색"));
  expect(host.textContent).toContain("명세 검색 실패: 검색 실패");
  const retries = [...host.querySelectorAll("button")].filter((element) => element.textContent === "다시 시도");
  await click(retries[0] as HTMLButtonElement);
  await click(button("명세 검색"));
  expect(host.textContent).toContain("일부만 표시");
});

it("shows a current-workspace link report without treating external links as verified", async () => {
  await render(); await searchAndRead();
  await click(button("문서 링크 검사"));
  expect(taskSpecValidateLinks).toHaveBeenCalledWith(1, "docs/auth.md", hash);
  expect(host.textContent).toContain("현재 워크트리 원문");
  expect(host.textContent).toContain("3행 · guide.md · 확인됨");
  expect(host.textContent).toContain("외부 링크 1개는 네트워크로 확인하지 않았습니다.");
  expect(host.textContent).not.toContain("명세 준수");
});

it("discards stale link reports when the source changes and keeps failures retryable", async () => {
  const pending = deferred<SpecLinkReport>();
  vi.mocked(taskSpecValidateLinks).mockReturnValueOnce(pending.promise).mockRejectedValueOnce(new Error("원본 명세가 변경되었습니다"));
  vi.mocked(taskSpecRead).mockResolvedValueOnce(liveDocument).mockResolvedValueOnce({ ...liveDocument, content_hash: otherHash, content: "changed source" });
  await render(); await searchAndRead();
  await click(button("문서 링크 검사"));
  await click(button("명세 검색"));
  await click(button("인증 명세"));
  await act(async () => pending.resolve(linkReport));
  expect(host.textContent).not.toContain("현재 워크트리 원문");
  await click(button("문서 링크 검사"));
  expect(host.textContent).toContain("문서 링크 검사 실패: 원본 명세가 변경되었습니다");
  expect(button("다시 시도").disabled).toBe(false);
});

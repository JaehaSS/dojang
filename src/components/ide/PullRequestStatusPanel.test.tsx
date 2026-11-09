// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PrStatusSnapshot } from "../../lib/github-pr-status";

const mocks = vi.hoisted(() => ({ get: vi.fn(), openUrl: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../lib/github-pr-status", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../lib/github-pr-status")>(),
  taskPrStatusGet: mocks.get,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));

import { PullRequestStatusPanel } from "./PullRequestStatusPanel";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;
const snapshot = (overrides: Partial<PrStatusSnapshot> = {}): PrStatusSnapshot => ({
  status: "ready", message: null, repository: "owner/repo", branch: "feature/pr", local_head: "local-sha", observed_at: 1_700_000_000,
  pr: {
    number: 42, url: "https://github.com/owner/repo/pull/42", title: "PR title", state: "open", draft: false,
    head_sha: "b".repeat(40), head_branch: "feature/pr", base_branch: "main", review_decision: "APPROVED", review_requests: ["reviewer"],
    checks: [{ name: "test", status: "pass", description: "", details_url: null }],
  },
  ...overrides,
});
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
};
const settle = async () => { await Promise.resolve(); await Promise.resolve(); };
const render = async (props: Partial<React.ComponentProps<typeof PullRequestStatusPanel>> = {}) => {
  await act(async () => { root.render(<PullRequestStatusPanel taskId={1} onAttachReview={() => {}} {...props} />); await settle(); });
};
const button = (label: string) => [...host.querySelectorAll("button")].find((node) => node.textContent === label);

beforeEach(() => {
  vi.clearAllMocks();
  mocks.get.mockResolvedValue(snapshot());
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

describe("PullRequestStatusPanel", () => {
  it("distinguishes an absent PR from an unavailable inspection", async () => {
    mocks.get.mockResolvedValueOnce(snapshot({ status: "no_pr", message: "브랜치에 PR이 없습니다.", pr: null }));
    await render();
    expect(host.textContent).toContain("연결된 풀 리퀘스트가 없습니다.");
    expect(host.textContent).toContain("브랜치에 PR이 없습니다.");
    mocks.get.mockResolvedValueOnce(snapshot({ status: "unavailable", message: "gh 인증이 필요합니다.", pr: null }));
    await act(async () => { root.render(<PullRequestStatusPanel taskId={2} onAttachReview={() => {}} />); await settle(); });
    expect(host.textContent).toContain("풀 리퀘스트 상태를 조회하지 못했습니다.");
    expect(host.textContent).toContain("gh 인증이 필요합니다.");
  });

  it("shows unknown checks and no checks without claiming a pass", async () => {
    mocks.get.mockResolvedValueOnce(snapshot({ pr: { ...snapshot().pr!, checks: [{ name: "provider status", status: "unknown", description: "알 수 없는 상태", details_url: null }] } }));
    await render();
    expect(host.textContent).toContain("미확인 · provider status");
    mocks.get.mockResolvedValueOnce(snapshot({ pr: { ...snapshot().pr!, checks: [] } }));
    await act(async () => { button("PR 새로고침")?.click(); await settle(); });
    expect(host.textContent).toContain("검사 기록이 없습니다. 통과를 확인할 수 없습니다.");
  });

  it("warns when local and PR heads differ and attaches failure context without a mutation", async () => {
    const attach = vi.fn();
    mocks.get.mockResolvedValueOnce(snapshot({ pr: { ...snapshot().pr!, checks: [{ name: "build", status: "fail", description: "compile failed", details_url: null }] } }));
    await render({ onAttachReview: attach });
    expect(host.textContent).toContain("로컬 HEAD와 PR HEAD가 다릅니다");
    await act(async () => { button("실패 검사를 검토 사유에 연결")?.click(); });
    expect(attach).toHaveBeenCalledTimes(1);
    expect(attach.mock.calls[0][0]).toContain(`PR HEAD: ${"b".repeat(40)}`);
    expect(mocks.get).toHaveBeenCalledTimes(1);
  });

  it("drops a late task result after a task switch", async () => {
    const first = deferred<PrStatusSnapshot>();
    const second = deferred<PrStatusSnapshot>();
    mocks.get.mockImplementation((id: number) => id === 1 ? first.promise : second.promise);
    await render();
    await act(async () => { root.render(<PullRequestStatusPanel taskId={2} onAttachReview={() => {}} />); await settle(); });
    await act(async () => { second.resolve(snapshot({ repository: "new/repo", pr: { ...snapshot().pr!, title: "new PR" } })); await settle(); });
    await act(async () => { first.resolve(snapshot({ repository: "old/repo", pr: { ...snapshot().pr!, title: "old PR" } })); await settle(); });
    expect(host.textContent).toContain("new PR");
    expect(host.textContent).not.toContain("old PR");
  });

  it("keeps one request during loading and blocks attachment while local review is busy", async () => {
    const pending = deferred<PrStatusSnapshot>();
    const attach = vi.fn();
    mocks.get.mockReturnValueOnce(pending.promise);
    await render({ disabled: true, onAttachReview: attach });
    expect(button("PR 새로고침")?.disabled).toBe(true);
    await act(async () => { button("PR 새로고침")?.click(); button("PR 새로고침")?.click(); });
    await render({ disabled: true, onAttachReview: attach });
    expect(mocks.get).toHaveBeenCalledTimes(1);
    await act(async () => { pending.resolve(snapshot({ pr: { ...snapshot().pr!, checks: [{ name: "build", status: "fail", description: "", details_url: null }] } })); await settle(); });
    expect(button("실패 검사를 검토 사유에 연결")?.disabled).toBe(true);
    await act(async () => { button("실패 검사를 검토 사유에 연결")?.click(); });
    expect(attach).not.toHaveBeenCalled();
    await render({ disabled: false, onAttachReview: attach });
    await act(async () => { button("실패 검사를 검토 사유에 연결")?.click(); });
    expect(attach).toHaveBeenCalledOnce();
    expect(mocks.get).toHaveBeenCalledTimes(1);
  });

  it("clears a prior success after a refresh error and allows a retry", async () => {
    await render();
    expect(host.textContent).toContain("PR title");
    mocks.get.mockRejectedValueOnce(new Error("network down"));
    await act(async () => { button("PR 새로고침")?.click(); await settle(); });
    expect(host.textContent).toContain("PR 상태 조회 실패: Error: network down");
    expect(host.textContent).not.toContain("PR title");
    mocks.get.mockResolvedValueOnce(snapshot({ pr: { ...snapshot().pr!, title: "retried PR" } }));
    await act(async () => { button("PR 새로고침")?.click(); await settle(); });
    expect(host.textContent).toContain("retried PR");
  });

  it("does not expose unsafe links to the opener", async () => {
    mocks.get.mockResolvedValueOnce(snapshot({ pr: { ...snapshot().pr!, url: "https://github.com/owner/repo/issues/42", checks: [{ name: "unsafe", status: "fail", description: "", details_url: "http://127.0.0.1/internal" }, { name: "safe", status: "pass", description: "", details_url: "https://ci.example.com/build/42" }] } }));
    await render();
    expect(button("GitHub에서 PR 열기")).toBeUndefined();
    expect(host.textContent).toContain("검사 세부 링크를 열 수 없습니다.");
    await act(async () => { button("검사 세부 정보 열기")?.click(); });
    expect(mocks.openUrl).toHaveBeenCalledWith("https://ci.example.com/build/42");
    mocks.openUrl.mockRejectedValueOnce(new Error("opener unavailable"));
    await act(async () => { button("검사 세부 정보 열기")?.click(); await settle(); });
    expect(host.textContent).toContain("링크를 열지 못했습니다: Error: opener unavailable");
  });
});

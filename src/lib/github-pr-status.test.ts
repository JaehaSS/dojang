// @vitest-environment jsdom

import { describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import {
  prFailureReviewContext,
  safeGithubPullRequestUrl,
  safePublicHttpUrl,
  taskPrStatusGet,
  type PrStatusSnapshot,
} from "./github-pr-status";

const snapshot = (overrides: Partial<PrStatusSnapshot> = {}): PrStatusSnapshot => ({
  status: "ready", message: null, repository: "owner/repo", branch: "feature/pr", local_head: "local-sha", observed_at: 1_700_000_000,
  pr: {
    number: 42, url: "https://github.com/owner/repo/pull/42", title: "PR title", state: "open", draft: false,
    head_sha: "a".repeat(40), head_branch: "feature/pr", base_branch: "main", review_decision: "APPROVED", review_requests: [],
    checks: [{ name: "test", status: "fail", description: "실패 설명", details_url: "https://ci.example.com/build/1" }],
  },
  ...overrides,
});

describe("github PR status API", () => {
  it("uses the task-local read-only status command", async () => {
    invoke.mockResolvedValue(snapshot());
    await taskPrStatusGet(17);
    expect(invoke).toHaveBeenCalledWith("task_pr_status_get", { id: 17 });
  });

  it("allows only public HTTP(S) links and narrows PR links to GitHub pull requests", () => {
    expect(safePublicHttpUrl("https://ci.example.com/build/1")).toBe("https://ci.example.com/build/1");
    for (const unsafe of ["javascript:alert(1)", "https://user:pass@ci.example.com/x", "http://localhost:3000/x", "http://127.0.0.1/x", "http://10.1.2.3/x", "http://[::1]/x", "https://host/x"]) {
      expect(safePublicHttpUrl(unsafe)).toBeNull();
    }
    expect(safeGithubPullRequestUrl("https://github.com/owner/repo/pull/42")).toBe("https://github.com/owner/repo/pull/42");
    expect(safeGithubPullRequestUrl("https://github.com/owner/repo/issues/42")).toBeNull();
    expect(safeGithubPullRequestUrl("https://example.com/owner/repo/pull/42")).toBeNull();
    expect(safeGithubPullRequestUrl("http://github.com/owner/repo/pull/42")).toBeNull();
    expect(safeGithubPullRequestUrl("https://github.com/owner/repo/pull/42?tab=checks")).toBeNull();
    expect(safeGithubPullRequestUrl("https://github.com/owner/other/pull/42", "owner/repo", 42)).toBeNull();
  });

  it("bounds failed-check context by UTF-8 bytes while retaining PR SHA, number, and time", () => {
    const value = snapshot({ pr: { ...snapshot().pr!, checks: [{ name: "huge", status: "fail", description: "🙂".repeat(10_000), details_url: "https://ci.example.com/" + "x".repeat(10_000) }] } });
    const context = prFailureReviewContext(value)!;
    expect(context).toContain("PR: #42");
    expect(context).toContain(`PR HEAD: ${"a".repeat(40)}`);
    expect(context).toContain("조회 시각:");
    expect(context).toContain("[일부 검사 세부 정보를 생략했습니다]");
    expect(context).not.toContain("\ufffd");
    expect(new TextEncoder().encode(context).length).toBeLessThanOrEqual(4_000);
  });

  it("does not create review context without a failed check", () => {
    expect(prFailureReviewContext(snapshot({ pr: { ...snapshot().pr!, checks: [{ name: "unknown", status: "unknown", description: "", details_url: null }] } }))).toBeNull();
  });

  it("does not leak decoder state between clipped contexts and bounds malformed metadata", () => {
    const huge = (description: string, repository = "owner/repo") => snapshot({ repository, pr: { ...snapshot().pr!, head_sha: "not-a-sha".repeat(1_000), checks: [{ name: "fail", status: "fail", description, details_url: null }] } });
    const korean = prFailureReviewContext(huge("한".repeat(10_000), "r".repeat(10_000)))!;
    const emoji = prFailureReviewContext(huge("🙂".repeat(10_000)))!;
    expect(korean).not.toContain("\ufffd");
    expect(emoji).not.toContain("\ufffd");
    expect(emoji).not.toContain("한");
    expect(emoji).toContain("PR HEAD: 알 수 없음");
    expect(new TextEncoder().encode(korean).length).toBeLessThanOrEqual(4_000);
    expect(new TextEncoder().encode(emoji).length).toBeLessThanOrEqual(4_000);
  });
});

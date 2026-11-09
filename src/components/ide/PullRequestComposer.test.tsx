// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
const api = vi.hoisted(() => ({ preview: vi.fn(), execute: vi.fn() }));
vi.mock("../../lib/pr-delivery", () => ({ taskPrPreview: api.preview, taskPrExecute: api.execute }));
import { PullRequestComposer } from "./PullRequestComposer";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let host: HTMLDivElement | null = null;
afterEach(() => { host?.remove(); host = null; vi.clearAllMocks(); });
const preview = () => ({ token: "token", repository: "acme/repo", branch: "feature/a", base: "main", head: "a".repeat(40), title: "제목", body: "본문", commits: [], verification_summary: "없음", template_path: null, expires_at: 1 });
async function render(taskId = 1) { host = document.createElement("div"); document.body.append(host); const root = createRoot(host); await act(async () => root.render(<PullRequestComposer taskId={taskId} />)); return root; }
const button = (name: string) => [...host!.querySelectorAll("button")].find((item) => item.textContent === name)!;
const change = async (selector: string, value: string) => { const field = host!.querySelector<HTMLInputElement | HTMLTextAreaElement>(selector)!; Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")!.set!.call(field, value); await act(async () => field.dispatchEvent(new Event("input", { bubbles: true }))); };

describe("PullRequestComposer", () => {
  it("invalidates the prepared token when either editable field changes", async () => {
    api.preview.mockResolvedValue(preview()); await render();
    await act(async () => button("PR 미리보기 만들기").click());
    expect(button("PR 만들기").disabled).toBe(false);
    await change('input[aria-label="PR 제목"]', "수정 제목");
    expect(button("PR 만들기").disabled).toBe(true);
    expect(host!.textContent).not.toContain("HEAD:");
  });

  it("sends only an explicit action and ignores a late reply after task changes", async () => {
    let complete!: (value: ReturnType<typeof preview>) => void;
    api.preview.mockReturnValue(new Promise((resolve) => { complete = resolve; }));
    const root = await render(1);
    await act(async () => button("PR 미리보기 만들기").click());
    await act(async () => root.render(<PullRequestComposer taskId={2} />));
    await act(async () => complete(preview()));
    expect(button("PR 만들기").disabled).toBe(true);
    api.preview.mockResolvedValue(preview());
    await act(async () => button("PR 미리보기 만들기").click());
    api.execute.mockResolvedValue({ ...preview(), action: "push", pr_number: null, url: null, message: "push 완료" });
    await act(async () => button("브랜치 push").click());
    expect(api.execute).toHaveBeenCalledWith(2, "token", "push");
  });
});

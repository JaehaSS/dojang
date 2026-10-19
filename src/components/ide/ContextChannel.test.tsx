// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ContextChannel, type ContextChannelBinding } from "./ContextChannel";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const binding = (over: Partial<ContextChannelBinding> = {}): ContextChannelBinding => ({
  agent: "codex", model: "observed-model", busy: false,
  observation: { contextTokens: 129_200, contextWindow: 258_400, observedAt: 1_725_000_000, source: "app_server" },
  events: [
    { kind: "user", text: "질문", prompt_metrics: { version: 1, user_bytes: 6, sent_user_bytes: 1600, instruction_bytes: 200, skill_added_bytes: 30, goal_added_bytes: 10, capsule_added_bytes: 20, vault_added_bytes: 0 } },
    { kind: "tool_use", name: "Read", tool_id: "read" },
    { kind: "tool_result", tool_use_id: "read", result_chars: 12000 },
  ],
  onOpenDetails: () => {}, ...over,
});

describe("ContextChannel summary", () => {
  it("keeps actual tokens, input bytes and collected calls in a compact card", () => {
    const html = renderToStaticMarkup(<ContextChannel context={binding()} density="rail" />);
    expect(html).toContain("129,200");
    expect(html).toContain("258,400");
    expect(html).toContain("약 50% 남음");
    expect(html).toContain("입력 1,600 B");
    expect(html).toContain("도구 1회");
    expect(html).toContain("파일 1회");
    expect(html).toContain("Codex 실행 이벤트 관측");
    expect(html).not.toContain('aria-label="컨텍스트 상세"');
  });
  it("pins the expanded-column summary and keeps unavailable usage explicit", () => {
    const html = renderToStaticMarkup(<ContextChannel context={binding({ observation: null })} density="panel" />);
    expect(html).toContain("sticky top-0");
    expect(html).toContain("사용량 미관측");
    expect(html).not.toContain('role="progressbar"');
  });
});

describe("ContextChannel interactions", () => {
  let container: HTMLDivElement;
  let root: Root;
  const button = (label: string) => container.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`);
  const summary = () => container.querySelector<HTMLElement>("[data-context-summary]");
  const details = () => container.querySelector('[aria-label="컨텍스트 상세"]');
  const render = async (context: ContextChannelBinding, density: "rail" | "panel" = "panel", active = true) => {
    await act(async () => root.render(<ContextChannel context={context} density={density} active={active} />));
  };
  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

  it("rail details enter the activity column; panel details toggle in place", async () => {
    const onOpenDetails = vi.fn();
    await render(binding({ onOpenDetails }), "rail");
    await act(async () => button("컨텍스트 상세 펼치기")?.click());
    expect(onOpenDetails).toHaveBeenCalledOnce();
    expect(details()).toBeNull();
    await render(binding(), "panel");
    await act(async () => button("컨텍스트 상세 펼치기")?.click());
    expect(details()?.textContent).toContain("12,000자");
    expect(details()?.textContent).toContain("200 B");
    expect(details()?.textContent).not.toContain("12,000토큰");
    await act(async () => button("컨텍스트 상세 접기")?.click());
    expect(details()).toBeNull();
  });

  it("handles a pending focus request only when visible and does not steal focus on later tab returns", async () => {
    const onFocusHandled = vi.fn();
    const context = binding({ focusRequest: 3, onFocusHandled });
    await render(context, "panel", false);
    expect(onFocusHandled).not.toHaveBeenCalled();
    await render(context);
    expect(document.activeElement).toBe(summary());
    expect(details()).not.toBeNull();
    expect(onFocusHandled).toHaveBeenCalledOnce();
    expect(onFocusHandled).toHaveBeenCalledWith(3);
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    outside.focus();
    await render(context, "panel", false);
    await render(context);
    expect(document.activeElement).toBe(outside);
    outside.remove();
  });

  it("defers history refresh while hidden or busy, then refreshes when the visible turn ends", async () => {
    let finish!: () => void;
    const onRefresh = vi.fn(() => new Promise<void>(resolve => { finish = resolve; }));
    await render(binding({ onRefresh }), "panel", false);
    expect(onRefresh).not.toHaveBeenCalled();
    await render(binding({ onRefresh, busy: true }));
    expect(onRefresh).not.toHaveBeenCalled();
    expect(button("컨텍스트 기록 새로고침")?.disabled).toBe(true);
    await render(binding({ onRefresh }));
    expect(onRefresh).toHaveBeenCalledOnce();
    expect(container.querySelector('[role="status"]')).not.toBeNull();
    await act(async () => finish());
    expect(container.querySelector('[role="status"]')).toBeNull();
  });

  it("reports query failure and supports retry without closing the channel", async () => {
    const onRefresh = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(undefined);
    await render(binding({ onRefresh }));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("기록을 불러오지 못했습니다");
    await act(async () => button("컨텍스트 기록 새로고침")?.click());
    expect(onRefresh).toHaveBeenCalledTimes(2);
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("ignores a previous visibility cycle's late query error", async () => {
    let rejectOld!: (error: Error) => void;
    const onRefresh = vi.fn().mockImplementationOnce(() => new Promise<void>((_resolve, reject) => { rejectOld = reject; })).mockResolvedValue(undefined);
    const context = binding({ onRefresh });
    await render(context);
    await render(context, "panel", false);
    await render(context);
    await act(async () => rejectOld(new Error("late")));
    expect(onRefresh).toHaveBeenCalledTimes(2);
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });
});

// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DebateView } from "./DebateView";
import type { DebateEventLike } from "./debate-rounds";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// 마크다운 렌더는 이 테스트의 책임이 아니다 — 면에 무엇이 실렸는지만 본다.
vi.mock("./Markdown", () => ({
  Markdown: ({ text }: { text: string }) => <div data-testid="md">{text}</div>,
}));

// 부분 mock이어야 한다 — 통째로 대체하면 이 트리 아래가 쓰는 다른 IPC까지 사라진다.
const ipc = vi.hoisted(() => ({ convoInterrupt: vi.fn(), debateEnd: vi.fn() }));
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/ipc")>()),
  convoInterrupt: ipc.convoInterrupt,
  debateEnd: ipc.debateEnd,
}));

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const events: DebateEventLike[] = [
  { kind: "user", text: "어느 쪽인가?" },
  { kind: "text", text: "A안을 권한다", speaker: "left" },
  { kind: "text", text: "전제가 틀렸다", speaker: "right" },
];

const view = (over: Partial<React.ComponentProps<typeof DebateView>> = {}) => (
  <DebateView
    taskId={7}
    events={events}
    roundCap={3}
    panes={[
      { agent: "claude", model: "sonnet-4.6" },
      { agent: "codex", model: "gpt-5.2" },
    ]}
    busy={false}
    onSend={() => true}
    onEnded={() => undefined}
    {...over}
  />
);

const render = async (node: React.ReactElement) => {
  await act(async () => root?.render(node));
};

const typeDraft = async (text: string) => {
  const input = container?.querySelector("textarea");
  if (!input) throw new Error("토론 입력이 없다");
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  await act(async () => {
    setter?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  return input;
};

const byText = (text: string) =>
  [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.trim() === text);

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  ipc.convoInterrupt.mockReset().mockResolvedValue(undefined);
  ipc.debateEnd.mockReset().mockResolvedValue(undefined);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.restoreAllMocks();
});

describe("DebateView", () => {
  it("토론에서도 저장된 도구 이미지·음성·리소스 결과를 표시한다", async () => {
    const open=vi.fn();
    await render(view({ onOpenLink:open, events:[...events,{kind:"tool_output",speaker:"right",tool_use_id:"codex-tool",contents:[
      {type:"text",text:"계산 결과 42"},{type:"image",url:"data:image/png;base64,AQ=="},{type:"audio",url:"data:audio/wav;base64,AQ=="},{type:"resource",uri:"https://example.com/result",title:"결과 열기"}
    ]}]}));
    expect(container?.textContent).toContain("계산 결과 42");
    expect(container?.querySelector('img[alt="도구 이미지 결과"]')).not.toBeNull();
    expect(container?.querySelector("audio")?.controls).toBe(true);
    expect(container?.querySelector("audio")?.autoplay).toBe(false);
    await act(async()=>byText("결과 열기")?.click());expect(open).toHaveBeenCalledWith("https://example.com/result");
  });
  it("면 헤더에 에이전트 이름·모델 배지와 발화자 접근성 레이블을 단다", async () => {
    await render(view());
    const left = container?.querySelector('[aria-label="좌측 발화자 Claude Code"]');
    const right = container?.querySelector('[aria-label="우측 발화자 Codex"]');
    expect(left?.textContent).toContain("sonnet-4.6");
    expect(right?.textContent).toContain("gpt-5.2");
    // 라운드 구분선은 목록 구분이 아니라 제목 수준이다 — 안 들리면 두 면이 한 흐름으로 읽힌다.
    const heading = container?.querySelector('[role="heading"]');
    expect(heading?.textContent).toContain("라운드 1/3");
  });

  it("라운드마다 각 면 첫머리에 발화자를 단다 — 헤더가 스크롤로 멀어져도 누가 말했는지 보인다", async () => {
    await render(view({ events: [...events, { kind: "user", text: "다음" }, { kind: "text", text: "B안", speaker: "left" }] }));
    const tags = [...(container?.querySelectorAll('[data-testid="debate-speaker"]') ?? [])].map((t) => t.textContent);
    expect(tags).toEqual(["Claudesonnet-4.6", "Codexgpt-5.2", "Claudesonnet-4.6", "Codexgpt-5.2"]);
  });

  it("3자면 면이 셋이고 위치 이름·발화자 표지가 자리마다 붙는다", async () => {
    await render(
      view({
        panes: [
          { agent: "claude", model: "sonnet-4.6" },
          { agent: "codex", model: "gpt-5.2" },
          { agent: "agy", model: null },
        ],
        events: [...events, { kind: "text", text: "둘 다 부족하다", speaker: "third" }],
      }),
    );
    expect(container?.querySelector('[aria-label="첫째 발화자 Claude Code"]')).not.toBeNull();
    expect(container?.querySelector('[aria-label="둘째 발화자 Codex"]')).not.toBeNull();
    expect(container?.querySelector('[aria-label^="셋째 발화자"]')).not.toBeNull();
    const tags = container?.querySelectorAll('[data-testid="debate-speaker"]') ?? [];
    expect(tags).toHaveLength(3);
    const bodies = [...(container?.querySelectorAll('[data-testid="md"]') ?? [])].map((t) => t.textContent);
    expect(bodies).toEqual(["A안을 권한다", "전제가 틀렸다", "둘 다 부족하다"]);
  });

  it("지금보다 자리가 많던 이전 토론의 셋째 발화도 화면에 남는다", async () => {
    await render(view({ events: [...events, { kind: "text", text: "옛 셋째 발화", speaker: "third" }] }));
    expect(container?.textContent).toContain("이전 토론의 다른 참여자");
    expect(container?.textContent).toContain("옛 셋째 발화");
  });

  it("구조화 질문은 발화자별 원래 자리와 미상 영역에 남긴다", async () => {
    const renderQuestion = (id: string) => <div data-testid={`question-${id}`}>{id}</div>;
    await render(view({
      renderQuestion,
      events: [
        { kind: "interaction", interaction_id: "preamble" },
        { kind: "user", text: "토론 시작" },
        { kind: "text", text: "좌측 앞", speaker: "left" },
        { kind: "interaction", interaction_id: "left", speaker: "left" },
        { kind: "text", text: "좌측 뒤", speaker: "left" },
        { kind: "text", text: "우측 앞", speaker: "right" },
        { kind: "interaction", interaction_id: "right", speaker: "right" },
        { kind: "text", text: "우측 뒤", speaker: "right" },
        { kind: "interaction", interaction_id: "common" },
        { kind: "interaction", interaction_id: "orphan", speaker: "third" },
      ],
    }));

    const question = (id: string) => container?.querySelector(`[data-testid="question-${id}"]`) as HTMLElement;
    const tag = (name: string) => [...(container?.querySelectorAll('[data-testid="debate-speaker"]') ?? [])]
      .find((element) => element.textContent?.includes(name));
    expect(tag("Claude")?.parentElement?.contains(question("left"))).toBe(true);
    expect(tag("Codex")?.parentElement?.contains(question("right"))).toBe(true);
    expect(question("preamble").compareDocumentPosition(container?.querySelector('[role="heading"]') as Node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(question("common").compareDocumentPosition(tag("Claude") as Node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const orphanLabel = [...(container?.querySelectorAll("div") ?? [])].find((element) => element.textContent === "이전 토론의 다른 참여자");
    expect(orphanLabel).toBeDefined();
    expect(orphanLabel!.compareDocumentPosition(question("orphan")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("원본 이벤트가 없는 대기 질문 상태도 토론 끝에 남긴다", async () => {
    await render(view({ interactionStatus: <div data-testid="unanchored-question">미연결 질문</div> }));
    expect(container?.querySelector('[data-testid="unanchored-question"]')?.textContent).toBe("미연결 질문");
  });

  it("라운드가 도는 동안 컴포저는 잠기고 중단만 남는다", async () => {
    await render(view({ busy: true }));
    const input = container?.querySelector("textarea");
    expect(input?.disabled).toBe(true);
    expect(byText("전송")).toBeUndefined();
    await act(async () => byText("중단")?.click());
    expect(ipc.convoInterrupt).toHaveBeenCalledWith(7);
  });

  it("보내는 쪽이 거절하면 초안을 지우지 않는다 — 삼킨 발화는 다시 칠 수 없다", async () => {
    const rejected = vi.fn(() => false);
    await render(view({ onSend: rejected }));
    const input = await typeDraft("이어서 물어볼 것");
    await act(async () => byText("전송")?.click());
    expect(rejected).toHaveBeenCalledWith("이어서 물어볼 것");
    expect(input.value).toBe("이어서 물어볼 것");

    const accepted = vi.fn(() => true);
    await render(view({ onSend: accepted }));
    await act(async () => byText("전송")?.click());
    expect(accepted).toHaveBeenCalled();
    expect(container?.querySelector("textarea")?.value).toBe("");
  });

  it("합의 배너에 결론 복사와 토론 끝내기가 있고, 끝내면 우측 자리를 버린다", async () => {
    const ended = vi.fn();
    await render(view({ events: [...events, { kind: "debate_ended", reason: "consensus" }], onEnded: ended }));
    expect(container?.textContent).toContain("합의했습니다");
    expect(byText("결론 복사")).toBeDefined();
    await act(async () => byText("토론 끝내기")?.click());
    expect(ipc.debateEnd).toHaveBeenCalledWith(7);
    expect(ended).toHaveBeenCalled();
  });

  it("실수로 켠 토론도 첫 발화 전에 끝낼 수 있고, 라운드가 도는 동안에는 중단을 먼저 가리킨다", async () => {
    const ended = vi.fn();
    await render(view({ events: [], onEnded: ended }));
    await act(async () => byText("토론 끝내기")?.click());
    expect(ipc.debateEnd).toHaveBeenCalledWith(7);
    expect(ended).toHaveBeenCalled();

    ipc.debateEnd.mockClear();
    await render(view({ busy: true }));
    expect(byText("토론 끝내기")?.disabled).toBe(true);
    expect(byText("중단")).toBeDefined();
  });

  it("상한으로 끝나면 배너 문구만 다르고 결론 복사는 없다 — 복사할 결론이 없다", async () => {
    await render(view({ events: [...events, { kind: "debate_ended", reason: "round_cap" }] }));
    expect(container?.textContent).toContain("합의하지 못했습니다");
    expect(byText("결론 복사")).toBeUndefined();
    // 끝난 뒤 컴포저는 되살아난다 — 다음 발화가 곧 계속이다.
    expect(container?.querySelector("textarea")?.disabled).toBe(false);
  });
});

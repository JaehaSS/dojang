// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { contextShrinkWarning } from "../../lib/context-window";
import type { Task } from "../../lib/ipc";
import { LOCAL_HOST } from "../../lib/transport";
import { SessionModelSwitch } from "./SessionModelSwitch";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// ModelPicker는 카탈로그·IPC를 스스로 끌어오므로 여기서는 onChange만 노출하는 껍데기로 세운다.
// 이 테스트의 책임은 "가드 → 경고 → 저장 → 낙관 갱신" 순서이지 피커의 렌더가 아니다.
const picker = vi.hoisted(() => ({
  agent: "",
  model: "",
  observedModel: undefined as string | null | undefined,
  placement: "",
  disabled: false,
  onChange: (_: string) => undefined as void,
}));
vi.mock("./ModelPicker", () => ({
  ModelPicker: (props: {
    agent: string;
    model: string;
    observedModel?: string | null;
    placement?: string;
    disabled?: boolean;
    onChange: (m: string) => void;
  }) => {
    picker.agent = props.agent;
    picker.model = props.model;
    picker.observedModel = props.observedModel;
    picker.placement = props.placement ?? "";
    picker.disabled = props.disabled ?? false;
    picker.onChange = props.onChange;
    return <div data-testid="picker" />;
  },
}));

// 부분 mock이어야 한다 — 모듈 전체를 대체하면 이 컴포넌트 트리 아래가 import하는 다른 IPC가
// 통째로 사라진다. 바꾸는 것은 세션 전환 IPC 둘뿐이다.
const ipc = vi.hoisted(() => ({ taskModelSet: vi.fn(), taskAgentSet: vi.fn(), debateStart: vi.fn(), taskServiceTierSet: vi.fn(), codexSpeedModels: vi.fn() }));
vi.mock("../../lib/ipc", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../lib/ipc")>()),
  taskModelSet: ipc.taskModelSet,
  taskAgentSet: ipc.taskAgentSet,
  debateStart: ipc.debateStart,
  taskServiceTierSet: ipc.taskServiceTierSet,
  codexSpeedModels: ipc.codexSpeedModels,
}));

let container: HTMLDivElement | null = null;
let root: Root | null = null;

const task = (over: Partial<Task> = {}): Task =>
  ({
    id: 7,
    repo: "/r",
    branch: "b",
    base: "dev",
    worktree_path: "/w",
    instruction: "i",
    state: "AwaitingReview",
    created_at: 0,
    updated_at: 0,
    agent: "claude",
    model: "opus[1m]",
    mode: "conversation",
    // 좌표는 호스트+id다. 기본은 로컬이고, 원격 케이스는 host를 넘겨 만든다.
    host: LOCAL_HOST,
    ...over,
  }) as Task;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  // 렌더가 실패해도 직전 테스트 값이 남아 통과하는 것을 막는다.
  picker.agent = "";
  picker.model = "";
  picker.observedModel = undefined;
  picker.placement = "";
  picker.onChange = () => undefined;
  ipc.taskModelSet.mockReset().mockResolvedValue(undefined);
  ipc.taskAgentSet.mockReset().mockResolvedValue(task({ agent: "codex", model: "gpt-5.6-terra" }));
  ipc.debateStart.mockReset().mockResolvedValue(undefined);
  ipc.codexSpeedModels.mockReset().mockResolvedValue(["gpt-6-astra"]);
  ipc.taskServiceTierSet.mockReset().mockResolvedValue(task({ agent: "codex", model: "gpt-6-astra", service_tier: "fast" }));
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  vi.restoreAllMocks();
});

const render = async (node: React.ReactElement) => {
  await act(async () => root?.render(node));
};

describe("SessionModelSwitch", () => {
  it("현재 세션 모델을 피커에 넘기고, 헤더에서는 아래로 열게 한다", async () => {
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    expect(picker.agent).toBe("claude");
    expect(picker.model).toBe("opus[1m]");
    // 위로 열면 44px 헤더 밖으로 잘려 항목을 고를 수 없다.
    expect(picker.placement).toBe("down");
  });

  it("관측된 실행 모델을 피커에 넘긴다 — 칩이 요청값 대신 보여줄 값", async () => {
    await render(
      <SessionModelSwitch
        task={task()}
        observedModel="claude-opus-5[1m]"
        onChanged={() => undefined}
      />,
    );
    expect(picker.observedModel).toBe("claude-opus-5[1m]");
  });

  it("관측이 아직 없으면 그대로 비워 넘긴다 — 없는 것을 지어내지 않는다", async () => {
    await render(<SessionModelSwitch task={task()} observedModel={null} onChanged={() => undefined} />);
    expect(picker.observedModel).toBeNull();
  });

  it("모델 오버라이드가 없으면 빈 문자열로 넘긴다 — 피커의 '기본' 상태", async () => {
    await render(<SessionModelSwitch task={task({ model: null })} onChanged={() => undefined} />);
    expect(picker.model).toBe("");
  });

  it("agent가 없거나 공백뿐이면 렌더하지 않는다 — 바꿀 대상이 없다", async () => {
    await render(<SessionModelSwitch task={task({ agent: null })} onChanged={() => undefined} />);
    expect(container?.querySelector('[data-testid="picker"]')).toBeNull();

    await render(<SessionModelSwitch task={task({ agent: "   " })} onChanged={() => undefined} />);
    expect(container?.querySelector('[data-testid="picker"]')).toBeNull();
  });

  it("같은 모델을 다시 고르면 아무것도 하지 않는다 — 확인 창도 IPC도 없다", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onChanged = vi.fn();
    await render(
      <SessionModelSwitch task={task()} contextTokens={300_000} onChanged={onChanged} />,
    );

    await act(async () => picker.onChange("  opus[1m]  "));

    expect(confirm).not.toHaveBeenCalled();
    expect(ipc.taskModelSet).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("윈도가 좁아지고 관측이 넘치면 확인을 받고, 취소하면 저장하지 않는다", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    const onChanged = vi.fn();
    await render(
      <SessionModelSwitch task={task()} contextTokens={300_000} onChanged={onChanged} />,
    );

    await act(async () => picker.onChange("opus"));

    // 세 인자(agent·next·current)의 배선을 한 번에 고정한다.
    expect(confirm).toHaveBeenCalledWith(
      contextShrinkWarning("claude", "opus", "opus[1m]", 300_000),
    );
    expect(ipc.taskModelSet).not.toHaveBeenCalled();
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("확인을 승인하면 저장하고 낙관 갱신을 알린다", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const onChanged = vi.fn();
    await render(
      <SessionModelSwitch task={task()} contextTokens={300_000} onChanged={onChanged} />,
    );

    await act(async () => picker.onChange("opus"));

    expect(ipc.taskModelSet).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, "opus");
    expect(onChanged).toHaveBeenCalledWith("opus");
  });

  it("윈도가 충분하면 확인 없이 저장한다", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onChanged = vi.fn();
    await render(
      <SessionModelSwitch task={task()} contextTokens={120_000} onChanged={onChanged} />,
    );

    await act(async () => picker.onChange("sonnet"));

    expect(confirm).not.toHaveBeenCalled();
    expect(ipc.taskModelSet).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, "sonnet");
  });

  it("저장이 실패하면 원인을 담아 알리고 낙관 갱신을 하지 않는다", async () => {
    const alert = vi.spyOn(window, "alert").mockImplementation(() => undefined);
    ipc.taskModelSet.mockRejectedValue("db 잠김");
    const onChanged = vi.fn();
    await render(<SessionModelSwitch task={task()} onChanged={onChanged} />);

    await act(async () => picker.onChange("sonnet"));

    expect(String(alert.mock.calls[0]?.[0])).toContain("db 잠김");
    expect(onChanged).not.toHaveBeenCalled();
  });

  it("실패해도 잠금이 풀려 다시 시도할 수 있다", async () => {
    vi.spyOn(window, "alert").mockImplementation(() => undefined);
    ipc.taskModelSet.mockRejectedValueOnce("일시 오류").mockResolvedValue(undefined);
    const onChanged = vi.fn();
    await render(<SessionModelSwitch task={task()} onChanged={onChanged} />);

    await act(async () => picker.onChange("sonnet"));
    await act(async () => picker.onChange("sonnet"));

    expect(ipc.taskModelSet).toHaveBeenCalledTimes(2);
    expect(onChanged).toHaveBeenCalledTimes(1);
  });

  it("저장이 진행 중이면 두 번째 조작을 삼킨다", async () => {
    let release: (() => void) | null = null;
    ipc.taskModelSet.mockReturnValue(
      new Promise<void>((resolve) => {
        release = () => resolve();
      }),
    );
    const onChanged = vi.fn();
    await render(<SessionModelSwitch task={task()} onChanged={onChanged} />);

    await act(async () => picker.onChange("sonnet"));
    await act(async () => picker.onChange("haiku"));

    expect(ipc.taskModelSet).toHaveBeenCalledTimes(1);
    expect(ipc.taskModelSet).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, "sonnet");

    await act(async () => {
      release?.();
    });
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(onChanged).toHaveBeenCalledWith("sonnet");
  });

  it("저장 성공 후 새 모델이 내려오면 피커가 그것을 보여준다", async () => {
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    await act(async () => picker.onChange("sonnet"));
    // 부모(App)가 setTasks로 갈아끼운 뒤의 재렌더를 흉내낸다.
    await render(<SessionModelSwitch task={task({ model: "sonnet" })} onChanged={() => undefined} />);
    expect(picker.model).toBe("sonnet");
  });

  it("대화용 에이전트만 고르고, 새 에이전트에서는 모델 초안을 비운다", async () => {
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    await act(async () => (container?.querySelector('button[title="다음 메시지를 처리할 에이전트"]') as HTMLButtonElement | null)?.click());
    expect(container?.textContent).toContain("Claude Code");
    expect(container?.textContent).toContain("Codex");
    expect(container?.textContent).toContain("Antigravity · Gemini");

    await act(async () => ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("Codex")) as HTMLButtonElement | undefined)?.click());
    expect(picker.agent).toBe("codex");
    expect(picker.model).toBe("");
    expect(container?.querySelector('button[type="button"]')?.hasAttribute("disabled")).toBe(true);
  });

  it("교차 에이전트 전환은 모델을 고르고 전환을 눌러야 저장 결과를 부모에 준다", async () => {
    const onChanged = vi.fn();
    await render(<SessionModelSwitch task={task()} onChanged={onChanged} />);
    await act(async () => (container?.querySelector('button[title="다음 메시지를 처리할 에이전트"]') as HTMLButtonElement | null)?.click());
    await act(async () => ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("Codex")) as HTMLButtonElement | undefined)?.click());
    await act(async () => picker.onChange("gpt-5.6-terra"));
    await act(async () => [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "전환")?.click());

    expect(ipc.taskAgentSet).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, "codex", "gpt-5.6-terra");
    expect(onChanged).toHaveBeenCalledWith("gpt-5.6-terra", expect.objectContaining({ agent: "codex" }));
  });

  it("교차 전환 실패는 현재 작업을 갱신하지 않고 재시도할 수 있다", async () => {
    const alert = vi.spyOn(window, "alert").mockImplementation(() => undefined);
    ipc.taskAgentSet.mockRejectedValueOnce("db 잠김").mockResolvedValueOnce(task({ agent: "codex", model: "gpt-5.6-terra" }));
    const onChanged = vi.fn();
    await render(<SessionModelSwitch task={task()} onChanged={onChanged} />);
    await act(async () => (container?.querySelector('button[title="다음 메시지를 처리할 에이전트"]') as HTMLButtonElement | null)?.click());
    await act(async () => ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("Codex")) as HTMLButtonElement | undefined)?.click());
    await act(async () => picker.onChange("gpt-5.6-terra"));
    const submit = () => [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "전환")?.click();
    await act(async () => submit());
    expect(alert).toHaveBeenCalledWith(expect.stringContaining("db 잠김"));
    expect(onChanged).not.toHaveBeenCalled();
    await act(async () => submit());
    expect(onChanged).toHaveBeenCalledTimes(1);
  });

  it.each(["Queued", "Running", "Done", "Failed", "Discarded"])("%s 상태는 에이전트 전환을 막고 이유를 보인다", async (state) => {
    await render(<SessionModelSwitch task={task({ state })} onChanged={() => undefined} />);
    expect(container?.querySelector('button[title*="검토 대기"]')?.hasAttribute("disabled")).toBe(true);
    expect(container?.textContent).toContain("대화 작업 검토 대기에서 전환 가능");
  });

  it("턴이 끝나 검토 대기로 갱신되면 에이전트 선택이 활성화된다", async () => {
    await render(<SessionModelSwitch task={task({ state: "Running" })} onChanged={() => undefined} />);
    expect(container?.querySelector("button")?.disabled).toBe(true);
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    const agentButton = container?.querySelector<HTMLButtonElement>('button[title="다음 메시지를 처리할 에이전트"]');
    expect(agentButton).not.toBeNull();
    expect(agentButton?.disabled).toBe(false);
    await act(async () => agentButton?.click());
    expect(container?.textContent).toContain("Codex");
  });

  it("교차 전환 요청이 진행 중이면 중복 제출하지 않는다", async () => {
    let release: (() => void) | null = null;
    ipc.taskAgentSet.mockReturnValue(new Promise<Task>((resolve) => { release = () => resolve(task({ agent: "codex" })); }));
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    await act(async () => (container?.querySelector('button[title="다음 메시지를 처리할 에이전트"]') as HTMLButtonElement | null)?.click());
    await act(async () => ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("Codex")) as HTMLButtonElement | undefined)?.click());
    await act(async () => picker.onChange("gpt-5.6-terra"));
    const submit = () => [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "전환")?.click();
    await act(async () => { submit(); submit(); });
    expect(ipc.taskAgentSet).toHaveBeenCalledTimes(1);
    await act(async () => release?.());
  });

  it("토론 시작은 전환과 같은 게이트다 — 검토 대기가 아니면 눌리지 않는다", async () => {
    await render(<SessionModelSwitch task={task({ state: "Running" })} onChanged={() => undefined} />);
    const debate = () => [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("토론 시작"));
    expect(debate()?.disabled).toBe(true);
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    expect(debate()?.disabled).toBe(false);
  });

  it("상대 피커에 현재 에이전트는 없다 — 자기 자신과는 토론하지 않는다", async () => {
    const started = vi.fn();
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} onDebateStarted={started} />);
    await act(async () =>
      ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("토론 시작")) as HTMLButtonElement | undefined)?.click(),
    );
    // 열린 메뉴는 토론 피커의 것이다 — 첫 버튼(손잡이)을 뺀 나머지가 고를 수 있는 상대다.
    const menu = [...(container?.querySelectorAll("div.relative") ?? [])][1];
    const options = [...menu.querySelectorAll("button")].slice(1);
    expect(options.map((b) => b.textContent)).toEqual([
      expect.stringContaining("Codex"),
      expect.stringContaining("Antigravity"),
      expect.stringContaining("모두 (3자 토론)"),
    ]);
    await act(async () => (options[0] as HTMLButtonElement).click());
    // 라운드 상한은 이 자리에서 묻지 않는다 — 상대만 넘긴다.
    expect(ipc.debateStart).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, [{ agent: "codex" }]);
    expect(started).toHaveBeenCalled();
  });

  it("\"모두\"를 고르면 남은 두 에이전트가 목록 순서대로 우측·셋째 자리를 받는다", async () => {
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    await act(async () =>
      ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("토론 시작")) as HTMLButtonElement | undefined)?.click(),
    );
    const all = [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("모두 (3자 토론)"));
    await act(async () => (all as HTMLButtonElement).click());
    expect(ipc.debateStart).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, [{ agent: "codex" }, { agent: "agy" }]);
  });

  it("토론 중에는 전환 자리를 닫는다 — 시퀀스 사이에도 작업은 검토 대기로 돌아온다", async () => {
    await render(<SessionModelSwitch task={task()} inDebate onChanged={() => undefined} />);
    const buttons = () => [...(container?.querySelectorAll("button") ?? [])];
    // 토론 시작은 사라진다 — 이미 도는 토론에 또 시작할 자리가 없다.
    expect(buttons().find((b) => b.textContent?.includes("토론 시작"))).toBeUndefined();
    // 남은 손잡이(에이전트 초안)와 모델 피커는 잠긴다 — 면마다 벤더 세션이 이미 하나씩 있다.
    expect(buttons().every((b) => b.disabled)).toBe(true);
    expect(picker.disabled).toBe(true);
    expect(container?.textContent).toContain("토론이 끝나면 전환할 수 있습니다");
  });

  it("대기 중인 전환 뒤 다른 작업을 열면 새 작업의 선택 초안을 보인다", async () => {
    let release: (() => void) | null = null;
    ipc.taskAgentSet.mockReturnValue(new Promise<Task>((resolve) => { release = () => resolve(task({ agent: "codex" })); }));
    await render(<SessionModelSwitch task={task()} onChanged={() => undefined} />);
    await act(async () => (container?.querySelector('button[title="다음 메시지를 처리할 에이전트"]') as HTMLButtonElement | null)?.click());
    await act(async () => ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("Codex")) as HTMLButtonElement | undefined)?.click());
    await act(async () => picker.onChange("gpt-5.6-terra"));
    await act(async () => ([...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "전환") as HTMLButtonElement | undefined)?.click());

    await render(<SessionModelSwitch task={task({ id: 8, model: "sonnet" })} onChanged={() => undefined} />);
    expect(picker.agent).toBe("claude");
    expect(picker.model).toBe("sonnet");
    await act(async () => release?.());
  });
  it("원격(SSH) 세션도 모델을 바꾼다 — 좌표에 그 호스트가 실린다", async () => {
    const onChanged = vi.fn();
    await render(<SessionModelSwitch task={task({ host: "mini1" })} onChanged={onChanged} />);
    // 모델 피커는 잠기지 않는다 — 원격에서 막히는 것은 에이전트 전환·토론뿐이다.
    expect(picker.disabled).toBe(false);
    await act(async () => picker.onChange("sonnet"));
    // id만 보내면 Runner의 7번이 아니라 같은 번호의 로컬 7번이 바뀐다. 호스트가 함께 가야 한다.
    expect(ipc.taskModelSet).toHaveBeenCalledWith({ host: "mini1", id: 7 }, "sonnet");
    expect(onChanged).toHaveBeenCalledWith("sonnet");
  });

  it("원격에서는 에이전트 전환·토론만 잠기고, 사유가 상태가 아니라 호스트를 가리킨다", async () => {
    await render(<SessionModelSwitch task={task({ host: "mini1" })} onChanged={() => undefined} />);
    const buttons = [...(container?.querySelectorAll("button") ?? [])];
    expect(buttons.length).toBeGreaterThan(0);
    expect(buttons.every((button) => button.disabled)).toBe(true);
    // "검토 대기를 기다리면 열린다"로 읽히면 사용자가 오지 않을 상태를 기다린다.
    expect(container?.textContent).not.toContain("대화 작업 검토 대기에서 전환 가능");
    expect(container?.textContent).toContain("로컬 세션에서만 쓸 수 있습니다");
    for (const button of buttons) {
      expect(button.getAttribute("title")).toContain("로컬 세션에서만 쓸 수 있습니다");
    }
  });

  it("원격에서는 토론을 시작하지 않는다 — 잠긴 손잡이를 눌러도 IPC로 새지 않는다", async () => {
    const started = vi.fn();
    await render(<SessionModelSwitch task={task({ host: "mini1" })} onChanged={() => undefined} onDebateStarted={started} />);
    const debate = [...(container?.querySelectorAll("button") ?? [])].find((button) => button.textContent?.includes("토론 시작"));
    expect(debate?.disabled).toBe(true);
    await act(async () => debate?.click());
    expect(ipc.debateStart).not.toHaveBeenCalled();
    expect(started).not.toHaveBeenCalled();
  });
});


describe("session speed", () => {
  it("saves the next turn's speed and publishes the returned task", async () => {
    const onServiceTierChanged = vi.fn();
    await render(<SessionModelSwitch task={task({ agent: "codex", model: "gpt-6-astra", service_tier: "default", state: "Running" })} onChanged={vi.fn()} onServiceTierChanged={onServiceTierChanged} />);
    const select = container!.querySelector<HTMLSelectElement>('[aria-label="Codex 실행 속도"]')!;
    await act(async () => { select.value="fast"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(ipc.taskServiceTierSet).toHaveBeenCalledWith({ host: LOCAL_HOST, id: 7 }, "fast");
    expect(onServiceTierChanged).toHaveBeenCalledWith(expect.objectContaining({service_tier:"fast"}));
  });
  it("reports storage failures without publishing a successful selection", async () => {
    ipc.taskServiceTierSet.mockRejectedValue(new Error("storage failure"));
    const alert=vi.spyOn(window,"alert").mockImplementation(()=>{});
    const onServiceTierChanged=vi.fn();
    await render(<SessionModelSwitch task={task({agent:"codex",model:"gpt-6-astra",service_tier:"default"})} onChanged={vi.fn()} onServiceTierChanged={onServiceTierChanged} />);
    const select=container!.querySelector<HTMLSelectElement>('[aria-label="Codex 실행 속도"]')!;
    await act(async()=>{select.value="fast";select.dispatchEvent(new Event("change",{bubbles:true}));});
    expect(alert).toHaveBeenCalledWith(expect.stringContaining("storage failure"));
    expect(onServiceTierChanged).not.toHaveBeenCalled();
    expect(select.value).toBe("default");
  });
  it("does not show local speed controls for remote, debate or other-provider tasks", async () => {
    for (const over of [{host:"remote"}, {agent:"claude"}, {mode:"terminal"}, {ensemble:"group"}]) {
      await render(<SessionModelSwitch task={task({agent:"codex",model:"gpt-6-astra",...over})} onChanged={vi.fn()} />);
      expect(container!.querySelector('[aria-label="Codex 실행 속도"]')).toBeNull();
    }
    await render(<SessionModelSwitch task={task({agent:"codex",model:"gpt-6-astra"})} inDebate onChanged={vi.fn()} />);
    expect(container!.querySelector('[aria-label="Codex 실행 속도"]')).toBeNull();
  });
});

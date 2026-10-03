// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  sessionHomeIndex: vi.fn(
    async (_host: unknown, _repo: unknown, _all?: unknown, _query?: unknown) => [] as unknown[],
  ),
  sessionHomePreview: vi.fn(async () => ({ meta: {} as never, messages: [], truncated: false })),
}));

vi.mock("../../lib/ipc", () => mocks);
vi.mock("./icons", () => ({ Icon: () => null }));

import { SessionHomePicker } from "./SessionHomePicker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement;
let root: Root;
const close = vi.fn();
const select = vi.fn();

function session(overrides: Record<string, unknown> = {}) {
  return {
    session_id: "abcd1234efgh5678",
    cwd: "/repo",
    last_cwd: null,
    git_branch: null,
    title: "테스트 세션",
    first_message: "첫 메시지",
    last_active: 0,
    messages: 5,
    vendor_version: null,
    host: "local",
    ...overrides,
  };
}

function render(projects: string[] = []) {
  root.render(
    <SessionHomePicker host="local" repo="/repo" projects={projects} onSelect={select} onClose={close} />,
  );
}

const sessionRow = (): HTMLButtonElement =>
  host.querySelector('button[data-kind="session"]') as HTMLButtonElement;
const projectRows = (): HTMLButtonElement[] =>
  Array.from(host.querySelectorAll<HTMLButtonElement>('button[data-kind="project"]'));

function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const key = (target: Element, key: string) =>
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));

const flush = async () => {
  await act(async () => vi.advanceTimersByTime(200));
  await act(async () => await Promise.resolve());
};

describe("SessionHomePicker", () => {
  beforeEach(() => {
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    close.mockClear();
    select.mockClear();
    mocks.sessionHomeIndex.mockReset();
    mocks.sessionHomePreview.mockReset();
    mocks.sessionHomeIndex.mockResolvedValue([]);
    mocks.sessionHomePreview.mockResolvedValue({ meta: session() as never, messages: [], truncated: false });
    vi.useFakeTimers();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  it("행에서 Esc는 상세를 먼저 닫고 두 번째에 선택창을 닫는다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([session()]);
    await act(async () => render());
    await flush();
    await act(async () => sessionRow().click());
    await act(async () => key(sessionRow(), "Escape"));
    expect(close).not.toHaveBeenCalled();
    expect(host.textContent).toContain("세션을 선택하면 최근 대화를 확인");
    await act(async () => key(sessionRow(), "Escape"));
    expect(close).toHaveBeenCalledOnce();
  });

  it("목록 조회 뒤 활동한 세션도 최신 미리보기 시각으로 경고한다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([session({ last_active: 0 })]);
    mocks.sessionHomePreview.mockResolvedValue({ meta: session({ last_active: Math.floor(Date.now() / 1000) }) as never, messages: [], truncated: false });
    await act(async () => render());
    await flush();
    await act(async () => sessionRow().click());
    const button = Array.from(host.querySelectorAll("button")).find((b) => b.textContent === "이 세션 이어받기")!;
    await act(async () => button.click());
    expect(select).not.toHaveBeenCalled();
    expect(host.textContent).toContain("최근에도 쓰인 세션");
  });

  it("세션을 선택한 뒤 명시적 이어받기로 확정한다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([session({ last_active: 0 })]);
    await act(async () => render());
    await flush();

    const row = sessionRow();
    expect(row?.textContent).toContain("테스트 세션");
    await act(async () => row?.click());

    expect(select).not.toHaveBeenCalled();
    const resume = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "이 세션 이어받기");
    await act(async () => resume?.click());
    expect(select).toHaveBeenCalledTimes(1);
    expect(select.mock.calls[0][0].session_id).toBe("abcd1234efgh5678");
  });

  it("제목이 없으면 첫 메시지를 쓰고, 평문 그대로 렌더한다(마크업 해석 없음)", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([
      session({ title: null, first_message: "<b>주입 시도</b>" }),
    ]);
    await act(async () => render());
    await flush();

    const row = sessionRow();
    expect(row?.querySelector("b")).toBeNull();
    expect(row?.textContent).toContain("<b>주입 시도</b>");
  });

  it("최근 활동 세션은 확인 없이 선택되지 않고 경고를 보여준다", async () => {
    const now = Math.floor(Date.now() / 1000);
    mocks.sessionHomeIndex.mockResolvedValue([session({ last_active: now - 10 })]);
    await act(async () => render());
    await flush();

    const row = sessionRow();
    await act(async () => row?.click());
    const resume = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "이 세션 이어받기");
    await act(async () => resume?.click());

    expect(select).not.toHaveBeenCalled();
    expect(host.textContent).toContain("다른 터미널에서 아직 열려 있으면");

    const confirm = Array.from(host.querySelectorAll("button")).find((b) => b.textContent === "이어받기");
    await act(async () => confirm?.click());
    expect(select).toHaveBeenCalledTimes(1);
  });

  it("다른 저장소의 세션은 확인을 요구한다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([
      session({ cwd: "/other-repo", last_active: 0 }),
    ]);
    await act(async () => render());
    await flush();

    // 다른 프로젝트는 접힌 채 열리므로 먼저 펼친다.
    expect(sessionRow()).toBeNull();
    await act(async () => projectRows()[0].click());
    const row = sessionRow();
    await act(async () => row?.click());
    const resume = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "이 세션 이어받기");
    await act(async () => resume?.click());

    expect(select).not.toHaveBeenCalled();
    expect(host.textContent).toContain("원래 작업 디렉터리가 지금 선택한 저장소와 다릅니다");
  });

  it("돌아가기를 누르면 목록으로 복귀하고 선택되지 않는다", async () => {
    const now = Math.floor(Date.now() / 1000);
    mocks.sessionHomeIndex.mockResolvedValue([session({ last_active: now })]);
    await act(async () => render());
    await flush();

    const row = sessionRow();
    await act(async () => row?.click());
    const resume = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "이 세션 이어받기");
    await act(async () => resume?.click());
    const back = Array.from(host.querySelectorAll("button")).find((b) => b.textContent === "돌아가기");
    await act(async () => back?.click());

    expect(select).not.toHaveBeenCalled();
    expect(sessionRow()).not.toBeNull();
  });

  it("다른 호스트로 태그된 세션은 선택되지 않는다(교차 호스트 제출 거절)", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([session({ host: "mini1", last_active: 0 })]);
    await act(async () => render());
    await flush();

    const row = sessionRow();
    await act(async () => row?.click());

    expect(select).not.toHaveBeenCalled();
  });

  it("저장소 조회와 전체 조회를 함께 보내 session_id로 합친다", async () => {
    mocks.sessionHomeIndex.mockImplementation(async (_host: unknown, _repo: unknown, all: unknown) =>
      all
        ? [session({ session_id: "other-1", cwd: "/elsewhere" }), session({ session_id: "mine-1" })]
        : [session({ session_id: "mine-1" })],
    );
    await act(async () => render());
    await flush();

    expect(mocks.sessionHomeIndex).toHaveBeenCalledWith("local", "/repo", false, undefined);
    expect(mocks.sessionHomeIndex).toHaveBeenCalledWith("local", "/repo", true, undefined);
    // 현재 저장소는 펼쳐진 채, 다른 프로젝트는 접힌 채 — 세션 행은 현재 저장소의 것 하나뿐.
    expect(projectRows().map((b) => b.getAttribute("aria-expanded"))).toEqual(["true", "false"]);
    expect(host.querySelectorAll('button[data-kind="session"]')).toHaveLength(1);
  });

  it("세션은 등록 프로젝트 아래에 묶이고, 현재 저장소 프로젝트가 맨 위다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([
      session({ session_id: "t", cwd: "/tool/.praxis/worktrees/x", last_active: 5000 }),
      session({ session_id: "r", cwd: "/repo/sub", last_active: 1 }),
    ]);
    await act(async () => render(["/tool"]));
    await flush();

    expect(projectRows().map((b) => b.textContent)).toEqual([
      expect.stringContaining("repo"),
      expect.stringContaining("tool"),
    ]);
    expect(projectRows()[1].textContent).toContain("/tool");
  });

  it("접힌 프로젝트를 누르면 펼쳐지고, 검색 중에는 전부 펼친다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([
      session({ session_id: "o", cwd: "/other", title: "다른 곳" }),
      session({ session_id: "m", title: "내 것" }),
    ]);
    await act(async () => render());
    await flush();
    expect(host.textContent).not.toContain("다른 곳");

    await act(async () => projectRows()[1].click());
    expect(host.textContent).toContain("다른 곳");
    await act(async () => projectRows()[1].click());
    expect(host.textContent).not.toContain("다른 곳");

    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;
    await act(async () => type(input, "다른"));
    await flush();
    expect(host.textContent).toContain("다른 곳");
  });

  it("↑↓로 행을 오가고 Enter는 프로젝트를 펼치거나 세션을 고른다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([
      session({ session_id: "o", cwd: "/other", title: "다른 곳", last_active: 0 }),
      session({ session_id: "m", title: "내 것", last_active: 0 }),
    ]);
    await act(async () => render());
    await flush();
    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;

    // 행 순서: repo(펼침) → 내 것 → other(접힘)
    await act(async () => key(input, "ArrowDown"));
    await act(async () => key(input, "ArrowDown"));
    expect(input.getAttribute("aria-activedescendant")).toBe("session-home-project:/other");
    await act(async () => key(input, "Enter"));
    expect(projectRows()[1].getAttribute("aria-expanded")).toBe("true");
    expect(select).not.toHaveBeenCalled();

    await act(async () => key(input, "ArrowDown"));
    expect(input.getAttribute("aria-activedescendant")).toBe("session-home-session:local:claude:o");
    await act(async () => key(input, "Enter"));
    // Enter는 상세만 열며, 이어받기는 상세 버튼에서 확정한다.
    expect(select).not.toHaveBeenCalled();
    expect(host.textContent).toContain("/other");
  });

  it("Esc는 목록에서 닫고, 확인 단계에서는 목록으로 돌아간다", async () => {
    const now = Math.floor(Date.now() / 1000);
    mocks.sessionHomeIndex.mockResolvedValue([session({ last_active: now })]);
    await act(async () => render());
    await flush();

    await act(async () => sessionRow().click());
    const resume = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "이 세션 이어받기");
    await act(async () => resume?.click());
    expect(host.textContent).toContain("이어받으시겠습니까");
    const dialog = host.querySelector('[role="dialog"]') as HTMLElement;
    await act(async () => key(dialog, "Escape"));
    expect(close).not.toHaveBeenCalled();
    expect(sessionRow()).not.toBeNull();

    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;
    await act(async () => key(input, "Escape"));
    expect(close).not.toHaveBeenCalled();
    expect(host.querySelector('[aria-label="세션 미리보기"]')?.textContent).toContain("선택하면");
    await act(async () => key(input, "Escape"));
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("방향키 선택은 미리보기를 바꾸고 Enter는 이어받기 버튼으로 포커스한다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([
      session({ session_id: "a", title: "첫 세션", recent_user_message: "첫 요청" }),
      session({ session_id: "b", title: "둘째 세션", recent_user_message: "둘째 요청" }),
    ]);
    mocks.sessionHomePreview.mockImplementation((async (_host: unknown, _repo: unknown, _vendor: unknown, id: unknown) => ({
      meta: session({ session_id: String(id), title: `${id} 전체 제목`, cwd: `/full/${id}` }),
      messages: [{ role: "user", text: `${id} 대화` }],
      truncated: false,
    })) as never);
    await act(async () => render());
    await flush();
    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;

    await act(async () => key(input, "ArrowDown"));
    await act(async () => await Promise.resolve());
    expect(host.querySelector('[aria-label="세션 미리보기"]')?.textContent).toContain("a 전체 제목");
    expect(host.querySelector('[aria-label="세션 미리보기"]')?.textContent).toContain("/full/a");

    await act(async () => key(input, "ArrowDown"));
    await act(async () => await Promise.resolve());
    expect(host.querySelector('[aria-label="세션 미리보기"]')?.textContent).toContain("b 대화");
    await act(async () => key(input, "Enter"));
    await act(async () => vi.advanceTimersByTime(20));
    expect(document.activeElement?.textContent).toBe("이 세션 이어받기");
  });

  it("미리보기 오류 중에는 이어받기를 막고 재시도 후에만 푼다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([session({ session_id: "a" })]);
    mocks.sessionHomePreview.mockRejectedValueOnce(new Error("forbidden"));
    mocks.sessionHomePreview.mockResolvedValueOnce({ meta: session({ session_id: "a" }) as never, messages: [], truncated: false });
    await act(async () => render());
    await flush();
    await act(async () => sessionRow().click());
    await act(async () => await Promise.resolve());
    const resume = Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent === "이 세션 이어받기");
    expect(resume?.getAttribute("aria-disabled")).toBe("true");
    const retry = Array.from(host.querySelectorAll<HTMLButtonElement>("button")).find((button) => button.textContent?.includes("다시 시도"));
    await act(async () => retry?.click());
    await act(async () => await Promise.resolve());
    expect(resume?.getAttribute("aria-disabled")).toBe("false");
  });

  it("두 조회가 모두 실패해야 오류를 보여주고, 하나만 실패하면 나머지로 그린다", async () => {
    mocks.sessionHomeIndex.mockImplementation(async (_h: unknown, _r: unknown, all: unknown) => {
      if (all) throw new Error("boom");
      return [session({ session_id: "m" })];
    });
    await act(async () => render());
    await flush();
    expect(sessionRow()).not.toBeNull();

    mocks.sessionHomeIndex.mockRejectedValue(new Error("boom"));
    await act(async () => root.unmount());
    root = createRoot(host);
    await act(async () => render());
    await flush();
    expect(host.textContent).toContain("세션 목록을 불러오지 못했습니다");
  });

  it("확인 단계에 들어가면 확인 버튼에, 돌아오면 입력창에 포커스가 간다", async () => {
    const now = Math.floor(Date.now() / 1000);
    mocks.sessionHomeIndex.mockResolvedValue([session({ last_active: now })]);
    await act(async () => render());
    await flush();

    await act(async () => sessionRow().click());
    const resume = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "이 세션 이어받기");
    await act(async () => resume?.click());
    await act(async () => vi.advanceTimersByTime(20));
    const confirm = Array.from(host.querySelectorAll("button")).find((b) => b.textContent === "이어받기");
    expect(document.activeElement).toBe(confirm);

    const back = Array.from(host.querySelectorAll("button")).find((b) => b.textContent === "돌아가기");
    await act(async () => back?.click());
    await act(async () => vi.advanceTimersByTime(20));
    expect(document.activeElement).toBe(host.querySelector('input[aria-label="세션 검색"]'));
  });

  it("첫 로드에 없던 프로젝트가 나중에 나타나도 접힌 채로 나온다", async () => {
    mocks.sessionHomeIndex.mockImplementation(async (_h: unknown, _r: unknown, all: unknown) => {
      if (all) throw new Error("boom");
      return [session({ session_id: "m", title: "내 것" })];
    });
    await act(async () => render());
    await flush();
    expect(projectRows()).toHaveLength(1);

    mocks.sessionHomeIndex.mockResolvedValue([
      session({ session_id: "o", cwd: "/other", title: "다른 곳" }),
      session({ session_id: "m", title: "내 것" }),
    ]);
    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;
    await act(async () => type(input, "것"));
    await flush();
    await act(async () => type(input, ""));
    await flush();

    expect(projectRows().map((b) => b.getAttribute("aria-expanded"))).toEqual(["true", "false"]);
    expect(host.textContent).not.toContain("다른 곳");
  });

  it("검색 중에는 커서가 첫 세션에 놓여 Enter로 바로 고른다", async () => {
    mocks.sessionHomeIndex.mockResolvedValue([session({ session_id: "m", title: "내 것", last_active: 0 })]);
    await act(async () => render());
    await flush();
    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;
    await act(async () => type(input, "내"));
    await flush();

    expect(input.getAttribute("aria-activedescendant")).toBe("session-home-session:local:claude:m");
    await act(async () => key(input, "Enter"));
    expect(select).not.toHaveBeenCalled();
  });

  it("검색어는 200ms 디바운스 뒤에 조회에 실린다", async () => {
    await act(async () => render());
    await flush();
    mocks.sessionHomeIndex.mockClear();

    const input = host.querySelector('input[aria-label="세션 검색"]') as HTMLInputElement;
    await act(async () => type(input, "foo"));
    expect(mocks.sessionHomeIndex).not.toHaveBeenCalled();
    await flush();
    expect(mocks.sessionHomeIndex).toHaveBeenCalledWith("local", "/repo", false, "foo");
    expect(mocks.sessionHomeIndex).toHaveBeenCalledWith("local", "/repo", true, "foo");
  });
});

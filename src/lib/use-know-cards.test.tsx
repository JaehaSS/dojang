// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { KnowCard } from "./ipc";
import { useKnowCards, type KnowCards } from "./use-know-cards";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
const handlers = new Map<string, (event: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(event, handler);
    return Promise.resolve(() => handlers.delete(event));
  },
}));

const card = (id: number, taskId: number, seen: number | null = null): KnowCard => ({
  id,
  task_id: taskId,
  kind: "heads_up",
  topic: `주제 ${id}`,
  title: `카드 ${id}`,
  body: "본문",
  evidence: "Edit src/a.ts",
  turn_label: "로그인 오류 고쳐줘",
  created_at: 1_700_000_000,
  seen_at: seen,
  asked_at: null,
});

let latest: KnowCards;
function Probe({ taskId }: { taskId: number | null }) {
  latest = useKnowCards(taskId);
  return <output>{latest.cards.map((c) => c.id).join(",")}</output>;
}

let root: Root;
let host: HTMLDivElement;
const text = () => host.querySelector("output")?.textContent ?? null;
const flush = async () => {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
};
const fire = async (payload: unknown) => {
  await act(async () => {
    handlers.get("know-cards://changed")?.({ payload });
    await Promise.resolve();
    await Promise.resolve();
  });
};
const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).map(([, args]) => args);

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
  handlers.clear();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

describe("useKnowCards", () => {
  it("선택한 세션의 카드를 읽고, 그 세션의 변경 이벤트에만 다시 읽는다", async () => {
    invoke.mockResolvedValueOnce([card(2, 7), card(1, 7, 100)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();
    expect(text()).toBe("2,1");
    expect(calls("know_cards_list")).toEqual([{ taskId: 7 }]);
    expect(latest.unread).toBe(1);

    await fire({ taskId: 8 });
    expect(calls("know_cards_list")).toHaveLength(1);

    invoke.mockResolvedValueOnce([card(3, 7), card(2, 7), card(1, 7, 100)]);
    await fire({ taskId: 7 });
    expect(text()).toBe("3,2,1");
    expect(latest.unread).toBe(2);
  });

  it("작업이 없으면 조회도 구독도 하지 않는다 — 원격·터미널 세션은 null로 온다", async () => {
    await act(async () => root.render(<Probe taskId={null} />));
    await flush();
    expect(text()).toBe("");
    expect(invoke).not.toHaveBeenCalled();
    expect(handlers.size).toBe(0);
  });

  it("세션이 바뀌면 새 목록이 오기 전에도 앞 세션의 카드를 내지 않는다", async () => {
    invoke.mockResolvedValueOnce([card(1, 7)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();
    expect(text()).toBe("1");

    let resolve: (cards: KnowCard[]) => void = () => {};
    invoke.mockReturnValueOnce(new Promise<KnowCard[]>((r) => { resolve = r; }));
    await act(async () => root.render(<Probe taskId={9} />));
    expect(text()).toBe("");
    expect(latest.unread).toBe(0);

    await act(async () => resolve([card(5, 9)]));
    expect(text()).toBe("5");
  });

  it("늦게 돌아온 앞 세션의 응답이 새 세션의 목록을 덮지 않는다", async () => {
    let resolveOld: (cards: KnowCard[]) => void = () => {};
    invoke.mockReturnValueOnce(new Promise<KnowCard[]>((r) => { resolveOld = r; }));
    await act(async () => root.render(<Probe taskId={7} />));
    invoke.mockResolvedValueOnce([card(5, 9)]);
    await act(async () => root.render(<Probe taskId={9} />));
    await flush();
    expect(text()).toBe("5");

    await act(async () => resolveOld([card(1, 7)]));
    expect(text()).toBe("5");
  });

  it("이미 앎·닫기는 응답 전에 카드를 빼고, 자세히는 카드를 남긴다", async () => {
    invoke.mockResolvedValueOnce([card(2, 7), card(1, 7)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();

    await act(async () => latest.act(2, "asked"));
    expect(text()).toBe("2,1");
    await act(async () => latest.act(2, "dismissed"));
    expect(text()).toBe("1");
    await act(async () => latest.act(1, "known"));
    expect(text()).toBe("");
    expect(calls("know_card_act")).toEqual([
      { cardId: 2, action: "asked" },
      { cardId: 2, action: "dismissed" },
      { cardId: 1, action: "known" },
    ]);
  });

  it("동작 직전에 떠난 조회가 뺀 카드를 되살리지 않는다", async () => {
    invoke.mockResolvedValueOnce([card(2, 7), card(1, 7)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();

    let resolve: (cards: KnowCard[]) => void = () => {};
    invoke.mockReturnValueOnce(new Promise<KnowCard[]>((r) => { resolve = r; }));
    await fire({ taskId: 7 });
    await act(async () => latest.act(2, "dismissed"));
    await act(async () => resolve([card(2, 7), card(1, 7)]));
    expect(text()).toBe("1");
  });

  it("동작 뒤에 떠난 조회도 뺀 카드를 되살리지 않는다 — 백엔드가 동작을 기록하기 전에 읽을 수 있다", async () => {
    invoke.mockResolvedValueOnce([card(2, 7), card(1, 7)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();

    let settle: () => void = () => {};
    invoke.mockImplementation(async (cmd) => {
      if (cmd === "know_card_act") return new Promise<void>((r) => { settle = r; });
      if (cmd === "know_cards_list") return [card(3, 7), card(2, 7), card(1, 7)];
      return undefined;
    });
    await act(async () => latest.act(2, "dismissed"));
    // 다른 판정이 새 카드를 남겼다 — 그 조회가 닫기보다 먼저 저장소를 읽었다.
    await fire({ taskId: 7 });
    expect(text()).toBe("3,1");
    await act(async () => settle());
  });

  it("동작이 실패하면 저장된 목록으로 되돌린다", async () => {
    invoke.mockResolvedValueOnce([card(2, 7), card(1, 7)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();

    invoke.mockImplementation(async (cmd) => {
      if (cmd === "know_card_act") throw new Error("db locked");
      if (cmd === "know_cards_list") return [card(2, 7), card(1, 7)];
      return undefined;
    });
    await act(async () => latest.act(2, "dismissed"));
    await flush();
    expect(text()).toBe("2,1");
  });

  it("읽음 처리는 안 읽은 수를 바로 줄이고 백엔드에 기록한다", async () => {
    invoke.mockResolvedValueOnce([card(2, 7), card(1, 7)]);
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();
    expect(latest.unread).toBe(2);

    await act(async () => latest.markSeen(2));
    expect(latest.unread).toBe(1);
    expect(latest.cards[0].seen_at).not.toBeNull();
    expect(calls("know_card_seen")).toEqual([{ cardId: 2 }]);
  });

  it("언마운트 뒤에는 구독을 풀고 늦은 응답을 버린다", async () => {
    let resolve: (cards: KnowCard[]) => void = () => {};
    invoke.mockReturnValueOnce(new Promise<KnowCard[]>((r) => { resolve = r; }));
    await act(async () => root.render(<Probe taskId={7} />));
    await flush();
    await act(async () => root.unmount());
    expect(handlers.size).toBe(0);
    resolve([card(1, 7)]);
    await flush();
    root = createRoot(host);
  });
});

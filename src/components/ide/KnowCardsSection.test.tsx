// @vitest-environment jsdom

import { act, type ComponentProps } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { KnowCard } from "../../lib/ipc";
import { KnowCardsSection, knowCardQuestion } from "./KnowCardsSection";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const at = (hours: number, minutes: number) => {
  const d = new Date();
  d.setHours(hours, minutes, 0, 0);
  return Math.floor(d.getTime() / 1000);
};

const card = (id: number, patch: Partial<KnowCard> = {}): KnowCard => ({
  id,
  task_id: 7,
  kind: "heads_up",
  topic: `주제 ${id}`,
  title: `카드 ${id}`,
  body: `본문 ${id}`,
  evidence: `Edit src/file${id}.ts`,
  turn_label: "로그인 오류 고쳐줘",
  created_at: at(14, 32),
  seen_at: null,
  asked_at: null,
  ...patch,
});

/** 최신 순 — 백엔드 `know_cards_list`와 같은 순서. */
const deck = (...ids: number[]) => ids.map((id) => card(id));

type Props = ComponentProps<typeof KnowCardsSection>;
let root: Root;
let host: HTMLDivElement;
let onAct: ReturnType<typeof vi.fn<Props["onAct"]>>;
let onSeen: ReturnType<typeof vi.fn<Props["onSeen"]>>;
let onAsk: ReturnType<typeof vi.fn<NonNullable<Props["onAsk"]>>>;

const render = async (cards: KnowCard[], patch: Partial<Props> = {}) => {
  await act(async () =>
    root.render(
      <KnowCardsSection cards={cards} onAct={onAct} onSeen={onSeen} onAsk={onAsk} visible {...patch} />,
    ),
  );
};
const shown = () => host.querySelector("h3")?.textContent ?? null;
const pageLabel = () => host.querySelector('nav[aria-label="알아둘 것 페이지"] > span')?.textContent ?? null;
const button = (name: string) =>
  [...host.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === name || b.getAttribute("aria-label")?.startsWith(name),
  ) ?? null;
const click = async (name: string) => {
  const target = button(name);
  expect(target, `${name} 버튼이 있어야 한다`).not.toBeNull();
  await act(async () => target!.click());
};
const fresh = () => host.querySelector('[data-testid="know-cards-fresh"]') != null;

beforeEach(() => {
  onAct = vi.fn();
  onSeen = vi.fn();
  onAsk = vi.fn();
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

describe("KnowCardsSection — 모양", () => {
  it("활성 카드가 없으면 섹션째 없다", async () => {
    await render([]);
    expect(host.innerHTML).toBe("");
  });

  it("종류 칩·출처 줄·제목·본문·근거를 한 장에 보인다", async () => {
    await render([card(1)]);
    const aside = host.querySelector('aside[aria-label="알아둘 것"]');
    expect(aside).not.toBeNull();
    expect(aside!.textContent).toContain("주의");
    expect(aside!.textContent).toContain("14:32 · '로그인 오류 고쳐줘'");
    expect(shown()).toBe("카드 1");
    expect(aside!.textContent).toContain("본문 1");
    expect(aside!.textContent).toContain("근거: Edit src/file1.ts");
  });

  it("긴 텍스트를 생략하지 않고 고정 높이의 내용 영역에서 스크롤해 읽는다", async () => {
    await render([card(1)]);
    const article = host.querySelector("article")!;
    expect(article.className).toContain("h-44");
    expect(article.className).toContain("min-h-0");
    expect(article.className).toContain("overflow-y-auto");
    expect(article.className).toContain("whitespace-pre-wrap");
    expect(article.className).toContain("break-words");
    expect(article.tabIndex).toBe(0);
    expect(article.querySelector('[class*="line-clamp"], .truncate')).toBeNull();
  });

  it("알아둘 것 종류는 그 이름의 칩을 단다", async () => {
    await render([card(1, { kind: "you_should_know" })]);
    expect(host.querySelector("aside")!.textContent).toContain("알아둘 것");
    expect(host.querySelector("aside")!.textContent).not.toContain("주의");
  });

  it("오늘 만든 카드가 아니면 날짜까지 보인다", async () => {
    const yesterday = at(9, 5) - 86_400;
    await render([card(1, { created_at: yesterday })]);
    const d = new Date(yesterday * 1000);
    const md = `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    expect(host.textContent).toContain(`${md} 09:05`);
  });
});

describe("KnowCardsSection — 페이지 넘김", () => {
  it("한 장이면 페이저가 없다", async () => {
    await render(deck(1));
    expect(pageLabel()).toBeNull();
    expect(button("더 최신 카드")).toBeNull();
  });

  it("1페이지가 가장 최신이고, 넘기면 더 오래된 카드가 나온다", async () => {
    await render(deck(3, 2, 1));
    expect(shown()).toBe("카드 3");
    expect(pageLabel()).toBe("1/3");
    expect(button("더 최신 카드")!.hasAttribute("disabled")).toBe(true);

    await click("더 오래된 카드");
    await click("더 오래된 카드");
    expect(shown()).toBe("카드 1");
    expect(pageLabel()).toBe("3/3");
    expect(button("더 오래된 카드")!.hasAttribute("disabled")).toBe(true);
  });

  it("1페이지를 보는 중 새 카드가 오면 새 카드를 보여 준다", async () => {
    await render(deck(2, 1));
    await render(deck(3, 2, 1));
    expect(shown()).toBe("카드 3");
    expect(pageLabel()).toBe("1/3");
    expect(fresh()).toBe(false);
  });

  it("다른 페이지를 보는 중 새 카드가 오면 보던 카드를 유지하고 ‹에 점을 찍는다", async () => {
    await render(deck(2, 1));
    await click("더 오래된 카드");
    expect(shown()).toBe("카드 1");

    await render(deck(3, 2, 1));
    expect(shown()).toBe("카드 1");
    expect(pageLabel()).toBe("3/3");
    expect(fresh()).toBe(true);
    expect(button("더 최신 카드")!.getAttribute("aria-label")).toContain("새 카드 있음");

    // 1페이지까지 돌아가야 점이 사라진다 — 새 카드를 본 것은 그때다.
    await click("더 최신 카드");
    expect(fresh()).toBe(true);
    await click("더 최신 카드");
    expect(shown()).toBe("카드 3");
    expect(fresh()).toBe(false);
  });

  it("보던 카드를 빼면 같은 자리의 더 오래된 카드, 마지막이었으면 바로 앞 카드를 보인다", async () => {
    await render(deck(3, 2, 1));
    await click("더 오래된 카드");
    expect(shown()).toBe("카드 2");
    await render(deck(3, 1));
    expect(shown()).toBe("카드 1");
    expect(pageLabel()).toBe("2/2");

    await render(deck(3));
    expect(shown()).toBe("카드 3");
    expect(pageLabel()).toBeNull();
  });

  it("빠진 카드 대신 보인 카드에 머문다 — 이어서 새 카드가 와도 자리가 유지된다", async () => {
    await render(deck(4, 3, 2, 1));
    await click("더 오래된 카드");
    await click("더 오래된 카드");
    expect(shown()).toBe("카드 2");
    await render(deck(4, 3, 1));
    expect(shown()).toBe("카드 1");

    await render(deck(5, 4, 3, 1));
    expect(shown()).toBe("카드 1");
    expect(pageLabel()).toBe("4/4");
  });

  it("보던 가장 오래된 카드가 새 카드에 밀려나도 가장 가까운 카드를 보이고 점을 찍는다", async () => {
    await render(deck(5, 4, 3, 2, 1));
    for (let i = 0; i < 4; i += 1) await click("더 오래된 카드");
    expect(shown()).toBe("카드 1");

    // 여섯째 카드가 생기며 가장 오래된 카드가 밀려났다(상한 5장).
    await render(deck(6, 5, 4, 3, 2));
    expect(shown()).toBe("카드 2");
    expect(pageLabel()).toBe("5/5");
    expect(fresh()).toBe(true);
  });
});

describe("KnowCardsSection — 읽음 처리", () => {
  it("보이지 않는 자리에 마운트된 카드는 읽음 처리하지 않는다", async () => {
    await render(deck(2, 1), { visible: false });
    expect(onSeen).not.toHaveBeenCalled();

    await render(deck(2, 1), { visible: true });
    expect(onSeen.mock.calls).toEqual([[2]]);
  });

  it("지금 페이지의 카드만 읽음 처리한다", async () => {
    await render(deck(3, 2, 1));
    expect(onSeen.mock.calls).toEqual([[3]]);
    await click("더 오래된 카드");
    expect(onSeen.mock.calls).toEqual([[3], [2]]);
  });

  it("이미 읽은 카드는 다시 기록하지 않는다", async () => {
    await render([card(1, { seen_at: 100 })]);
    expect(onSeen).not.toHaveBeenCalled();
  });

  it("창이 가려져 있으면 다시 보일 때까지 미룬다", async () => {
    let state: DocumentVisibilityState = "hidden";
    // 인스턴스에 덮어쓴다 — 지우면 프로토타입의 getter가 다시 보인다.
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
    try {
      await render(deck(1));
      expect(onSeen).not.toHaveBeenCalled();

      state = "visible";
      await act(async () => document.dispatchEvent(new Event("visibilitychange")));
      expect(onSeen.mock.calls).toEqual([[1]]);
    } finally {
      delete (document as { visibilityState?: unknown }).visibilityState;
    }
  });
});

describe("KnowCardsSection — 동작", () => {
  it("자세히는 카드를 질문으로 넘기고 asked를 기록한다", async () => {
    const one = card(1);
    await render([one]);
    await click("자세히");
    expect(onAsk).toHaveBeenCalledWith(one);
    expect(onAct.mock.calls).toEqual([[1, "asked"]]);
  });

  it("사이드 질문을 쓸 수 없는 세션이면 자세히가 없다", async () => {
    await render(deck(1), { onAsk: undefined });
    expect(button("자세히")).toBeNull();
    expect(button("이미 앎")).not.toBeNull();
  });

  it("이미 앎과 닫기는 보고 있는 카드에 그 동작을 기록한다", async () => {
    await render(deck(2, 1));
    await click("더 오래된 카드");
    await click("이미 앎");
    await click("닫기");
    expect(onAct.mock.calls).toEqual([
      [1, "known"],
      [1, "dismissed"],
    ]);
  });

  it("더블클릭의 두 번째 클릭은 버린다 — 첫 클릭에 빠진 카드 자리로 다음 카드의 같은 버튼이 온다", async () => {
    const press = async (name: string, detail: number) => {
      const target = button(name);
      expect(target, `${name} 버튼이 있어야 한다`).not.toBeNull();
      await act(async () => target!.dispatchEvent(new MouseEvent("click", { bubbles: true, detail })));
    };
    await render(deck(3, 2, 1));
    await press("닫기", 1);
    await render(deck(2, 1)); // 부모가 카드를 뺐다
    await press("닫기", 2);
    expect(onAct.mock.calls).toEqual([[3, "dismissed"]]);

    // 키보드로 누르면 detail이 0이다 — 연달아 눌러도 한 번씩 받는다.
    await press("이미 앎", 0);
    expect(onAct.mock.calls).toEqual([
      [3, "dismissed"],
      [2, "known"],
    ]);
  });

  it("질문 맥락은 제목·본문·근거를 그대로 담는다", () => {
    expect(knowCardQuestion(card(4))).toBe("카드 4\n\n본문 4\n\n근거: Edit src/file4.ts");
  });
});

// @vitest-environment jsdom

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Badge, Button, Dialog, IconButton, TOOLTIP_OPEN_DELAY } from ".";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("Button", () => {
  it("기본 type은 button이다 — form 안에서 의도치 않게 제출하지 않는다", () => {
    expect(renderToStaticMarkup(<Button>확인</Button>)).toContain('type="button"');
  });

  it("variant마다 다른 표면을 쓴다", () => {
    expect(renderToStaticMarkup(<Button variant="primary">a</Button>)).toContain("bg-primary");
    expect(renderToStaticMarkup(<Button variant="danger">a</Button>)).toContain("text-status-failed");
    expect(renderToStaticMarkup(<Button variant="ghost">a</Button>)).not.toContain("border-border");
  });
});

describe("IconButton", () => {
  it("이름을 aria-label에 달고, 네이티브 title은 비활성일 때만 남긴다", () => {
    const html = renderToStaticMarkup(<IconButton label="닫기">x</IconButton>);
    expect(html).toContain('aria-label="닫기"');
    expect(html).not.toContain("title=");
    expect(html).not.toContain("aria-pressed");
    expect(renderToStaticMarkup(<IconButton label="닫기" disabled>x</IconButton>)).toContain('title="닫기"');
  });

  it("토글이면 aria-pressed와 중립 배경면을 단다 — 눌림을 teal로 칠하지 않는다", () => {
    const html = renderToStaticMarkup(<IconButton label="고정" pressed>p</IconButton>);
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain("bg-text/[.10]");
    expect(html).not.toMatch(/(^|\s)(text|bg)-primary/);
  });

  it("danger는 hover에서만 빨강이다 — 평소에는 다른 아이콘 버튼과 같은 색이다", () => {
    const html = renderToStaticMarkup(<IconButton label="삭제" tone="danger">x</IconButton>);
    expect(html).toContain("hover:text-status-failed");
    expect(html).toMatch(/(^|\s)text-text-secondary(\s|")/);
  });

  it("메뉴 버튼은 열린 동안 aria-expanded와 눌린 배경면을 달고 aria-pressed는 달지 않는다", () => {
    const html = renderToStaticMarkup(<IconButton label="더보기" expanded>m</IconButton>);
    expect(html).toContain('aria-expanded="true"');
    expect(html).toContain("bg-text/[.10]");
    expect(html).not.toContain("aria-pressed");
  });

  it("accent는 눌림 없이 글자색만 바꾼다", () => {
    const html = renderToStaticMarkup(<IconButton label="저장" tone="accent">s</IconButton>);
    expect(html).toContain("text-primary-bright");
    expect(html).not.toContain("aria-pressed");
    expect(html).not.toContain("text-text-secondary");
  });
});

describe("Dialog", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  it("Esc와 배경 mousedown에서 닫히고, 패널 안 mousedown에서는 닫히지 않는다", () => {
    const onClose = vi.fn();
    act(() => root.render(<Dialog label="테스트" onClose={onClose}><p>본문</p></Dialog>));
    const dialog = host.querySelector('[role="dialog"]')!;
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(dialog.getAttribute("aria-label")).toBe("테스트");

    act(() => void dialog.querySelector("p")!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    expect(onClose).not.toHaveBeenCalled();

    act(() => void dialog.parentElement!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("닫히면 연 버튼으로 포커스를 돌려준다", () => {
    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button id="opener" onClick={() => setOpen(true)}>열기</button>
          {open && (
            <Dialog label="d" onClose={() => setOpen(false)}>
              <input id="inside" autoFocus />
            </Dialog>
          )}
        </>
      );
    }
    act(() => root.render(<Harness />));
    const opener = host.querySelector<HTMLButtonElement>("#opener")!;
    opener.focus();
    act(() => opener.click());
    host.querySelector<HTMLInputElement>("#inside")!.focus();
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(host.querySelector('[role="dialog"]')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});

describe("Badge", () => {
  it("상태 색과 모양 아이콘을 문구와 함께 달고, 아이콘은 장식으로 숨긴다", () => {
    const html = renderToStaticMarkup(<Badge tone="failed">실패</Badge>);
    expect(html).toContain("text-status-failed");
    expect(html).toContain("rounded-full");
    expect(html).toContain('aria-hidden="true"');
    expect(html).not.toContain('role="img"');
    expect(html).toContain("실패");
  });
});

describe("IconButton 툴팁", () => {
  let host: HTMLDivElement;
  let root: Root;
  const tip = () => document.body.querySelector('[role="tooltip"]');
  const hover = (el: Element, type: "mouseover" | "mouseout") =>
    act(() => void el.dispatchEvent(new MouseEvent(type, { bubbles: true, relatedTarget: type === "mouseover" ? document.body : null })));

  beforeEach(() => {
    vi.useFakeTimers();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  it("hover는 지연 뒤에 띄우고, 떠나면 닫는다", () => {
    act(() => root.render(<IconButton label="새로고침">r</IconButton>));
    const btn = host.querySelector("button")!;
    hover(btn, "mouseover");
    expect(tip()).toBeNull();
    act(() => void vi.advanceTimersByTime(TOOLTIP_OPEN_DELAY));
    expect(tip()?.textContent).toBe("새로고침");
    // 내용이 접근 이름과 같으면 설명으로 잇지 않는다 — 같은 말을 두 번 읽는다.
    expect(btn.getAttribute("aria-describedby")).toBeNull();
    hover(btn, "mouseout");
    expect(tip()).toBeNull();
  });

  it("키보드 포커스에는 바로 띄우고 Esc로 닫는다. 마우스로 눌러 생긴 포커스에는 띄우지 않는다", () => {
    act(() => root.render(<IconButton label="저장" title="저장 (⌘S)">s</IconButton>));
    const btn = host.querySelector("button")!;
    act(() => btn.focus());
    expect(tip()?.textContent).toBe("저장 (⌘S)");
    expect(btn.getAttribute("aria-describedby")).toBe(tip()!.id);
    act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })));
    expect(tip()).toBeNull();

    act(() => btn.blur());
    act(() => void btn.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    act(() => btn.focus());
    act(() => void vi.advanceTimersByTime(TOOLTIP_OPEN_DELAY));
    expect(tip()).toBeNull();
  });

  it("호출부의 핸들러도 그대로 불린다", () => {
    const onMouseDown = vi.fn();
    const onFocus = vi.fn();
    act(() => root.render(<IconButton label="x" onMouseDown={onMouseDown} onFocus={onFocus}>x</IconButton>));
    const btn = host.querySelector("button")!;
    act(() => void btn.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    act(() => btn.focus());
    expect(onMouseDown).toHaveBeenCalledTimes(1);
    expect(onFocus).toHaveBeenCalledTimes(1);
  });
});

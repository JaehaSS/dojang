// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SkillMeta } from "../../lib/ipc";
import { SkillDropdown } from "./SkillDropdown";
import { MentionDropdown } from "./MentionDropdown";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const skills = Array.from({ length: 20 }, (_, i) => ({ name: `skill-${i}`, description: "" }) as SkillMeta);

let host: HTMLDivElement;
let root: Root;
let scrolled: string[];

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  scrolled = [];
  // jsdom에는 scrollIntoView가 없다 — 어느 행이 끌려왔는지만 기록한다.
  Element.prototype.scrollIntoView = vi.fn(function (this: Element) {
    scrolled.push(this.textContent ?? "");
  });
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
  delete (Element.prototype as Partial<Element>).scrollIntoView;
});

describe("드롭다운 선택 스크롤", () => {
  it("스킬 목록에서 선택이 옮겨지면 그 행을 보이는 데로 끌어온다", () => {
    const render = (sel: number) =>
      act(() => root.render(<SkillDropdown open items={skills} sel={sel} onHover={() => {}} onSelect={() => {}} />));
    render(0);
    render(15);
    expect(scrolled[scrolled.length - 1]).toContain("/skill-15");
  });

  it("멘션 목록에서도 선택 행을 끌어온다", () => {
    const items = Array.from({ length: 20 }, (_, i) => ({ kind: "file" as const, path: `src/f${i}.ts` }));
    act(() =>
      root.render(<MentionDropdown open items={items} sel={12} token="" onHover={() => {}} onSelect={() => {}} />),
    );
    expect(scrolled[scrolled.length - 1]).toContain("src/f12.ts");
  });
});

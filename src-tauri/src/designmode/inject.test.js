// @vitest-environment jsdom
//
// inject.js는 번들러 없이 웹뷰에 raw text로 주입되는 스크립트다. 여기서는 실제 주입과 동일한
// 실행 경로(원문을 그대로 `new Function`으로 현재 global에서 실행)로 순수 로직을 검증한다.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, beforeEach, describe, expect, it } from "vitest";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = readFileSync(path.join(here, "inject.js"), "utf-8");

describe("designmode inject.js", () => {
  let dm;

  // 실제 웹뷰에서도 페이지당 1회만 주입되므로, 리스너 중복 등록을 피하려고 스크립트는 한 번만 로드한다.
  beforeAll(() => {
    // eslint-disable-next-line no-new-func
    new Function(source)();
    dm = window.__praxisDesignMode;
  });

  beforeEach(() => {
    document.documentElement.innerHTML = "<body><button id=\"target\">Click</button></body>";
    dm.setEnabled(false);
  });

  it("raw selector injection alone does not mark a page as a native canvas", () => {
    expect(window.__dojangCanvasNative).toBeUndefined();
  });

  it("whitelistComputedStyle은 화이트리스트 속성만 남긴다", () => {
    const computed = {
      getPropertyValue: (prop) => (prop === "color" ? "rgb(0, 0, 0)" : prop === "__proto__" ? "polluted" : ""),
    };
    const out = dm._internal.whitelistComputedStyle(computed, dm._internal.CSS_WHITELIST);
    expect(out.color).toBe("rgb(0, 0, 0)");
    expect(out).not.toHaveProperty("__proto__");
  });

  it("toBoundingRect는 x/y/width/height만 순수 데이터로 뽑는다", () => {
    const rect = { x: 1, y: 2, width: 3, height: 4, extra: "ignored" };
    expect(dm._internal.toBoundingRect(rect)).toEqual({ x: 1, y: 2, width: 3, height: 4 });
  });

  it("truncateHtml은 상한 이하는 그대로 둔다", () => {
    expect(dm._internal.truncateHtml("<div>ok</div>")).toBe("<div>ok</div>");
  });

  it("truncateHtml은 상한을 넘으면 잘라내고 표시를 남긴다", () => {
    const huge = "x".repeat(20500);
    const out = dm._internal.truncateHtml(huge);
    expect(out.length).toBeLessThan(huge.length);
    expect(out).toContain("truncated");
  });

  it("serializeElement는 snake_case 3요소(outer_html/computed_css/bounding_rect)를 담는다", () => {
    const el = document.getElementById("target");
    const computed = { getPropertyValue: (prop) => (prop === "display" ? "flex" : "") };
    const payload = dm._internal.serializeElement(el, computed);
    expect(payload.outer_html).toContain("Click");
    expect(payload.computed_css.display).toBe("flex");
    expect(payload.bounding_rect).toEqual(expect.objectContaining({ x: expect.any(Number) }));
  });

  it("buildCaptureUrl은 praxis-designmode 스킴 + JSON 인코딩 쿼리를 만든다", () => {
    const url = dm._internal.buildCaptureUrl({ a: 1 });
    expect(url.startsWith("praxis-designmode:capture?data=")).toBe(true);
    const decoded = JSON.parse(decodeURIComponent(url.split("data=")[1]));
    expect(decoded).toEqual({ a: 1 });
  });

  it("highlightStyleFor는 rect 기준 fixed 오버레이 스타일을 만든다", () => {
    const style = dm._internal.highlightStyleFor({ left: 10, top: 20, width: 30, height: 40 });
    expect(style.position).toBe("fixed");
    expect(style.left).toBe("10px");
    expect(style.pointerEvents).toBe("none");
  });

  it("setEnabled(true) 후 mousemove로 하이라이트 박스가 생기고, false로 끄면 사라진다", () => {
    dm.setEnabled(true);
    const el = document.getElementById("target");
    el.dispatchEvent(new window.MouseEvent("mousemove", { bubbles: true }));
    expect(document.getElementById("__praxis_designmode_highlight__")).not.toBeNull();
    dm.setEnabled(false);
    expect(document.getElementById("__praxis_designmode_highlight__")).toBeNull();
  });

  it("selection mode가 꺼져 있으면 mousemove가 하이라이트를 만들지 않는다", () => {
    const el = document.getElementById("target");
    el.dispatchEvent(new window.MouseEvent("mousemove", { bubbles: true }));
    expect(document.getElementById("__praxis_designmode_highlight__")).toBeNull();
  });

  it("요소를 클릭하면 선택 모드는 내려가되 하이라이트는 남는다", () => {
    // 네이티브 스크린샷이 뒤늦게 찍히므로 하이라이트가 남아 있어야 캡처 지점이 이미지에 표시된다.
    // 동시에 enabled를 내려 이후 mousemove가 하이라이트를 다른 요소로 옮기지 못하게 고정한다.
    dm.setEnabled(true);
    const el = document.getElementById("target");
    el.dispatchEvent(new window.MouseEvent("mousemove", { bubbles: true }));
    el.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
    expect(dm.isEnabled()).toBe(false);
    expect(document.getElementById("__praxis_designmode_highlight__")).not.toBeNull();
  });

  it("isEnabled는 setEnabled 상태를 반영한다", () => {
    expect(dm.isEnabled()).toBe(false);
    dm.setEnabled(true);
    expect(dm.isEnabled()).toBe(true);
  });

  it("캔버스 선택기는 iframe 문서의 정적 태그와 CSS/rect를 camelCase로 전달한다", () => {
    document.documentElement.innerHTML = [
      "<body><button id=\"target\" class=\"p-4 text-sm\"",
      " data-praxis-src=\"src/design/card.tsx:12:4\"",
      " data-praxis-class-kind=\"static\">Click</button></body>"
    ].join("");
    const selected = [];
    const controller = dm.bindCanvasDocument(document, (payload) => selected.push(payload));
    controller.setEnabled(true);
    document.getElementById("target").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));

    expect(selected).toHaveLength(1);
    expect(selected[0]).toEqual(expect.objectContaining({
      version: 1,
      source: "src/design/card.tsx:12:4",
      className: "p-4 text-sm",
      classKind: "static",
      tagName: "button",
      suggestedAncestor: null,
      computedCss: expect.any(Object),
      boundingRect: expect.objectContaining({ x: expect.any(Number), width: expect.any(Number) })
    }));
    expect(document.getElementById("__praxis_designmode_highlight__")).not.toBeNull();
    controller.dispose();
  });

  it("태그 없는 클릭은 대상은 유지하고 가장 가까운 태그 조상만 제안한다", () => {
    document.documentElement.innerHTML = [
      "<body><section id=\"ancestor\" class=\"flex\"",
      " data-praxis-src=\"src/design/card.tsx:9:2\" data-praxis-class-kind=\"dynamic\">",
      "<span id=\"target\" class=\"inner\">Click</span></section></body>"
    ].join("");
    const selected = [];
    const controller = dm.bindCanvasDocument(document, (payload) => selected.push(payload));
    controller.setEnabled(true);
    document.getElementById("target").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));

    expect(selected[0]).toEqual(expect.objectContaining({
      source: null,
      className: "inner",
      classKind: "unknown",
      tagName: "span",
      suggestedAncestor: {
        instanceSource: null,
        source: "src/design/card.tsx:9:2",
        className: "flex",
        classKind: "dynamic",
        tagName: "section"
      }
    }));
    expect(controller.selectSuggestedAncestor()).toEqual(expect.objectContaining({
      source: "src/design/card.tsx:9:2",
      suggestedAncestor: null
    }));
    expect(selected).toHaveLength(2);
    controller.dispose();
  });

  it("캔버스 클릭은 한 번만 선택하고 dispose 뒤에는 문서 리스너를 제거한다", () => {
    const selected = [];
    const controller = dm.bindCanvasDocument(document, (payload) => selected.push(payload));
    const target = document.getElementById("target");
    controller.setEnabled(true);
    target.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
    target.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
    expect(selected).toHaveLength(1);
    controller.dispose();
    controller.setEnabled(true);
    target.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
    expect(selected).toHaveLength(1);
  });
});

// @vitest-environment jsdom

import { act, useEffect, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { LspTarget } from "../../lib/ipc";
import { fileTabKey, type TabKey } from "../../lib/tab-key";

const h = vi.hoisted(() => ({
  reveals: [] as Array<Record<string, unknown>>,
  deliveries: [] as Array<{
    navId: string;
    epoch: string;
    paneId: number;
    activePath: string;
    revealPath: string;
    matchesActivePath: boolean;
  }>,
  nextPaneId: 0,
  references: new Map<string, { complete: (targets: LspTarget[]) => void }>(),
}));

const target = (path: string): LspTarget => ({
  path,
  abs_path: `/workspace/${path}`,
  line: 8,
  column: 3,
  external: false,
});

const position = (path: string) =>
  ({
    "a.ts": { line: 11, column: 2, scrollTop: 101, scrollLeft: 7 },
    "b.ts": { line: 22, column: 3, scrollTop: 202, scrollLeft: 8 },
    "c.ts": { line: 33, column: 4, scrollTop: 303, scrollLeft: 9 },
  })[path] ?? { line: 1, column: 1, scrollTop: 0, scrollLeft: 0 };

vi.mock("./EditorPane", async () => {
  const React = await import("react");
  return {
    EditorPane: (props: {
      files: Array<{ path: string; key: string }>;
      activeKey: string | null;
      reveal?: Record<string, unknown> | null;
      onRevealed?: (ack: Record<string, unknown>) => void;
      onNavigateTarget?: (target: LspTarget, origin: Record<string, unknown>) => void;
      beginReferences?: () => number;
      onReferences?: (targets: LspTarget[], sourcePath: string, request: number, origin?: Record<string, unknown>) => void;
      onSelect: (key: string) => void;
      onSplit: (axis: "row" | "column") => void;
      onCloseGroup?: () => void;
    }) => {
      const paneId = React.useRef(h.nextPaneId++).current;
      const lastAck = React.useRef("");
      const deliveries = React.useRef(new Map<string, number>());
      const active = props.files.find((file) => file.key === props.activeKey)?.path ?? "";
      const revealKey = props.reveal == null ? "" : `${props.reveal.epoch}:${props.reveal.navId}`;
      useEffect(() => {
        if (!revealKey || !props.reveal) return;
        const path = String(props.reveal.path);
        const previous = deliveries.current.get(revealKey);
        const delivery = previous == null ? h.deliveries.length : previous;
        if (previous == null) {
          deliveries.current.set(revealKey, delivery);
          h.deliveries.push({
            navId: String(props.reveal.navId),
            epoch: String(props.reveal.epoch),
            paneId,
            activePath: active,
            revealPath: path,
            matchesActivePath: path === active,
          });
        } else {
          h.deliveries[delivery] = {
            ...h.deliveries[delivery],
            activePath: active,
            matchesActivePath: path === active,
          };
        }
        if (path !== active || lastAck.current === revealKey) return;
        lastAck.current = revealKey;
        const actual = position(path);
        const ack = { ...props.reveal, path, ...actual };
        h.reveals.push(ack);
        props.onRevealed?.(ack);
      }, [active, revealKey]);
      const origin = { path: active, ...position(active) };
      const next = active === "a.ts" ? "b.ts" : active === "b.ts" ? "c.ts" : "a.ts";
      return React.createElement(
        "section",
        { "data-pane-active": active },
        React.createElement("button", { "aria-label": `split-${active}`, onClick: () => props.onSplit("row") }),
        props.files.map((file) =>
          React.createElement("button", {
            key: file.key,
            "aria-label": `select-${active}-${file.path}`,
            onClick: () => props.onSelect(file.key),
          }),
        ),
        React.createElement("button", { "aria-label": `navigate-${active}`, onClick: () => props.onNavigateTarget?.(target(next), origin) }),
        React.createElement("button", { "aria-label": `navigate-same-${active}`, onClick: () => props.onNavigateTarget?.({ ...target(active), line: 8, column: 3 }, origin) }),
        props.onCloseGroup && React.createElement("button", { "aria-label": `close-${active}`, onClick: props.onCloseGroup }),
        React.createElement("button", {
          "aria-label": `references-${active}`,
          onClick: () => {
            const request = props.beginReferences?.() ?? 0;
            h.references.set(active, {
              complete: (targets) => props.onReferences?.(targets, active, request, origin),
            });
          },
        }),
      );
    },
  };
});

vi.mock("./ReferencesPanel", () => ({
  ReferencesPanel: ({ targets, stale, onOpen }: { targets: LspTarget[]; stale: boolean; onOpen: (target: LspTarget) => void }) => (
    <div data-reference-path={targets[0]?.path ?? ""} data-reference-stale={String(stale)}>
      <button aria-label="open-reference" onClick={() => onOpen(targets[0])} />
    </div>
  ),
}));

vi.mock("../../lib/ipc", async (original) => ({
  ...(await original<Record<string, unknown>>()),
  fsRead: vi.fn(),
}));

import { EditorSplitView } from "./EditorSplitView";
import type { OpenFile } from "./EditorPane";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const file = (path: string): OpenFile => ({
  key: fileTabKey(path), path, kind: "text", content: "source", baseContent: "source", mtime: 0, dirty: false,
});

let sourceChange: { current: (path: string) => void };

function Harness({ initial = ["a.ts", "b.ts", "c.ts"] }: { initial?: string[] }) {
  const [files, setFiles] = useState(() => initial.map(file));
  const [activeKey, setActiveKey] = useState<TabKey | null>(fileTabKey(initial[0] ?? "a.ts"));
  sourceChange = { current: () => undefined };
  return <EditorSplitView
    taskId={1} host="local" windowId="main" files={files} activeKey={activeKey} dark={false} ownsWindow
    onSelect={setActiveKey} onClose={(key) => setFiles((current) => current.filter((item) => item.key !== key))}
    onChange={() => undefined} onSave={() => undefined} onReload={() => undefined} onOpenPath={() => undefined} onRevealPath={() => undefined}
    onOpenTarget={async (next) => {
      if (next.path == null) return "failed";
      const path = next.path;
      setFiles((current) => current.some((item) => item.path === path) ? current : [...current, file(path)]);
      setActiveKey(fileTabKey(path));
      return "opened";
    }}
    sourceChangeRef={sourceChange}
  />;
}

let container: HTMLDivElement;
let root: Root;

const render = async (initial?: string[]) => {
  await act(async () => root.render(<Harness initial={initial} />));
};
const click = async (selector: string) => {
  const element = container.querySelector<HTMLButtonElement>(selector);
  expect(element, `missing ${selector}`).toBeTruthy();
  await act(async () => element?.click());
};

beforeEach(() => {
  h.reveals.length = 0;
  h.deliveries.length = 0;
  h.nextPaneId = 0;
  h.references.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("EditorSplitView navigation integration", () => {
  it("keeps only the latest reference result across two panes and stales only its source", async () => {
    await render(["a.ts", "b.ts"]);
    await click('[aria-label="split-a.ts"]');
    await click('[aria-label="select-a.ts-b.ts"]');
    await click('[aria-label="references-b.ts"]');
    await click('[aria-label="references-a.ts"]');

    await act(async () => h.references.get("a.ts")?.complete([target("new.ts")]));
    await act(async () => h.references.get("b.ts")?.complete([target("old.ts")]));

    expect(container.querySelector("[data-reference-path]")?.getAttribute("data-reference-path")).toBe("new.ts");
    await act(async () => sourceChange.current("b.ts"));
    expect(container.querySelector("[data-reference-path]")?.getAttribute("data-reference-stale")).toBe("false");
    await act(async () => sourceChange.current("a.ts"));
    expect(container.querySelector("[data-reference-path]")?.getAttribute("data-reference-stale")).toBe("true");
  });

  it("restores A's captured cursor and scroll after A to B to C then two back commands", async () => {
    await render();
    await click('[aria-label="navigate-a.ts"]');
    await click('[aria-label="navigate-b.ts"]');
    const back = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "뒤로");
    await act(async () => back?.click());
    await act(async () => back?.click());

    expect(h.reveals[h.reveals.length - 1]).toMatchObject({ path: "a.ts", line: 11, column: 2, scrollTop: 101, scrollLeft: 7, restore: true });
  });

  it("moves forward for Ctrl+Shift+- from a pane that stops keyboard bubbling", async () => {
    await render();
    await click('[aria-label="navigate-a.ts"]');
    await click('[aria-label="navigate-b.ts"]');
    const back = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "뒤로");
    await act(async () => back?.click());
    const pane = container.querySelector<HTMLElement>("[data-pane-active]")!;
    pane.addEventListener("keydown", (event) => event.stopPropagation());
    const event = new KeyboardEvent("keydown", {
      bubbles: true,
      cancelable: true,
      code: "Minus",
      key: "_",
      ctrlKey: true,
      shiftKey: true,
    });
    await act(async () => pane.dispatchEvent(event));

    expect(h.reveals[h.reveals.length - 1]).toMatchObject({ path: "c.ts", restore: true });
    expect(event.defaultPrevented).toBe(true);
  });

  it("delivers a same-path reveal to one pane and falls back when its original group closes", async () => {
    await render(["a.ts", "b.ts"]);
    await click('[aria-label="split-a.ts"]');
    const navigate = container.querySelectorAll<HTMLButtonElement>('[aria-label="navigate-same-a.ts"]');
    await act(async () => navigate[1]?.click());

    expect(h.reveals).toHaveLength(1);
    expect(h.deliveries.filter((delivery) => delivery.matchesActivePath)).toHaveLength(1);
    const close = container.querySelectorAll<HTMLButtonElement>('[aria-label="close-a.ts"]');
    await act(async () => close[1]?.click());
    const back = Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "뒤로");
    await act(async () => back?.click());

    expect(h.reveals[h.reveals.length - 1]).toMatchObject({ path: "a.ts", restore: true });
  });

  it("routes a reference through navigation", async () => {
    await render(["a.ts", "b.ts"]);
    await click('[aria-label="references-a.ts"]');
    await act(async () => h.references.get("a.ts")?.complete([target("b.ts")]));
    await click('[aria-label="open-reference"]');

    expect(Array.from(container.querySelectorAll("button")).find((button) => button.textContent === "뒤로")?.disabled).toBe(false);
  });
});

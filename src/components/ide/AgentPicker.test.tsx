// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { AgentPicker } from "./AgentPicker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  act(() => root.unmount());
  host.remove();
});

const buttons = () => [...host.querySelectorAll("button")];

it("1.0에서는 비교 실행(앙상블) 선택지가 없고, 고르면 한 에이전트로 교체된다", () => {
  const onChange = vi.fn();
  act(() => root.render(<AgentPicker agents={["claude"]} onChange={onChange} />));
  act(() => buttons()[0].click());

  expect(host.textContent).not.toContain("비교");
  act(() => buttons().find((b) => b.textContent?.includes("Codex"))!.click());
  expect(onChange).toHaveBeenLastCalledWith(["codex"]);
});

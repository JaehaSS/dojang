// @vitest-environment jsdom

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DesignCaptureRecord } from "../../lib/designmode/types";

const mocks = vi.hoisted(() => ({
  removeCapture: vi.fn(async () => undefined),
  skillsList: vi.fn(async () => []),
}));

vi.mock("../../lib/ipc", () => ({
  designmodeRemoveCapture: mocks.removeCapture,
  skillsList: mocks.skillsList,
}));


import { AgentComposer } from "./AgentComposer";
import { clearCaptures, getCaptures, pushCapture } from "../../lib/designmode/store";
import { requestComposerFocus } from "../../lib/composer-focus";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const capture: DesignCaptureRecord = {
  id: "1-0",
  task_id: 7,
  source: "editor",
  outer_html: "",
  computed_css: {},
  bounding_rect: { x: 10, y: 20, width: 600, height: 400 },
  captured_at: 1,
  image_path: "/tmp/editor.png",
  file_path: "src/App.tsx",
  selection_text: "const selected = true;",
  selection_start_line: 10,
  selection_end_line: 10,
};

let container: HTMLDivElement | null = null;
let root: Root | null = null;

beforeEach(() => {
  clearCaptures(7);
  pushCapture(7, capture);
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  clearCaptures(7);
  container?.remove();
  root = null;
  container = null;
  vi.clearAllMocks();
});

describe("AgentComposer capture attachments", () => {
  it("adds capture context to the prompt and forwards image paths on send", async () => {
    const onSend = vi.fn();
    function Harness() {
      const [value, setValue] = useState("이 코드를 검토해줘");
      return (
        <AgentComposer
          value={value}
          onChange={setValue}
          onSend={onSend}
          onInterrupt={() => {}}
          onHistory={() => {}}
          files={[]}
          repo="/repo"
          taskId={7}
        host="local"
        />
      );
    }
    await act(async () => root?.render(<Harness />));

    expect(container?.textContent).toContain("App.tsx L10");
    const send = container?.querySelector<HTMLButtonElement>('button[aria-label="전송"]');
    await act(async () => {
      send?.click();
      await Promise.resolve();
    });

    // 캡처 블록은 입력창을 경유하지 않고 전송 인자로 바로 간다. 경유하면 그 사이 도착한
    // 다른 전송(에디터 창 질문 등)이 사용자가 타이핑 중이던 내용을 덮어쓴다.
    expect(onSend).toHaveBeenCalledOnce();
    const [text, imagePaths] = onSend.mock.calls[0] as [string, string[]];
    expect(text).toContain("이 코드를 검토해줘");
    expect(text).toContain("파일: src/App.tsx");
    expect(text).toContain("const selected = true;");
    expect(imagePaths).toEqual(["/tmp/editor.png"]);

    // 입력창은 컴포저가 비우지 않는다 — 전송이 가드(busy·빈 입력)에 걸렸을 때 입력을 잃지 않게.
    const textarea = container?.querySelector<HTMLTextAreaElement>("textarea");
    expect(textarea?.value).toBe("이 코드를 검토해줘");
  });

  it("⌘L 첨부 직후 요청이 오면 입력창에 포커스를 준다", async () => {
    function Harness() {
      const [value, setValue] = useState("");
      return (
        <AgentComposer
          value={value}
          onChange={setValue}
          onSend={() => {}}
          onInterrupt={() => {}}
          onHistory={() => {}}
          files={[]}
          repo="/repo"
          taskId={7}
        host="local"
        />
      );
    }
    await act(async () => root?.render(<Harness />));
    const textarea = container?.querySelector<HTMLTextAreaElement>("textarea");
    expect(document.activeElement).not.toBe(textarea);

    await act(async () => requestComposerFocus(7));

    expect(document.activeElement).toBe(textarea);
  });

  it("다른 작업의 포커스 요청은 이 입력창을 건드리지 않는다", async () => {
    function Harness() {
      const [value, setValue] = useState("");
      return (
        <AgentComposer
          value={value}
          onChange={setValue}
          onSend={() => {}}
          onInterrupt={() => {}}
          onHistory={() => {}}
          files={[]}
          repo="/repo"
          taskId={7}
        host="local"
        />
      );
    }
    await act(async () => root?.render(<Harness />));

    await act(async () => requestComposerFocus(8));

    expect(document.activeElement).not.toBe(container?.querySelector("textarea"));
  });

  it("숨은 컴포저는 포커스 요청을 받지 않는다", async () => {
    await act(async () => {
      root?.render(
        <AgentComposer
          value=""
          onChange={() => {}}
          onSend={() => {}}
          onInterrupt={() => {}}
          onHistory={() => {}}
          files={[]}
          taskId={7}
          host="local"
          active={false}
        />,
      );
    });

    await act(async () => requestComposerFocus(7));

    expect(document.activeElement).not.toBe(container?.querySelector("textarea"));
  });
});


describe("capture admission retention", () => {
  async function mount(onSend: (text: string, images: string[]) => Promise<boolean>, host = "local") {
    await act(async () => root?.render(<AgentComposer value="apply" onChange={() => {}} onSend={onSend} onInterrupt={() => {}} onHistory={() => {}} files={[]} taskId={7} host={host} />));
  }
  it("retains captures on rejected admission", async () => {
    await mount(async () => false);
    await act(async () => container?.querySelector<HTMLButtonElement>('button[aria-label="전송"]')?.click());
    expect(getCaptures(7)).toEqual([capture]);
  });
  it("consumes only submitted captures after acknowledgement", async () => {
    let resolve!: (accepted: boolean) => void;
    const onSend = vi.fn(() => new Promise<boolean>((done) => { resolve = done; }));
    await mount(onSend);
    await act(async () => container?.querySelector<HTMLButtonElement>('button[aria-label="전송"]')?.click());
    expect(getCaptures(7)).toEqual([capture]);
    const next = { ...capture, id: "later" };
    await act(async () => pushCapture(7, next));
    await act(async () => resolve(true));
    expect(getCaptures(7)).toEqual([next]);
  });
  it("does not attach local captures to the same numeric task on a remote host", async () => {
    const onSend = vi.fn(async () => true);
    await mount(onSend, "remote");
    await act(async () => container?.querySelector<HTMLButtonElement>('button[aria-label="전송"]')?.click());
    expect(onSend).toHaveBeenCalledWith("apply", []);
    expect(getCaptures(7)).toEqual([capture]);
  });
});

describe("queued composer submission", () => {
  it("allows Enter and the queue button to submit with attachments while keeping Shift+Enter and IME local", async () => {
    const onSend = vi.fn(async () => true);
    await act(async () => root?.render(<AgentComposer value="다음 요청" onChange={() => {}} onSend={onSend}
      sendLabel="대기열에 추가" onInterrupt={() => {}} onHistory={() => {}} files={[]} taskId={7} host="local" />));
    const textarea = container!.querySelector("textarea")!;
    const send = container!.querySelector<HTMLButtonElement>('button[aria-label="대기열에 추가"]')!;
    expect(send.disabled).toBe(false);
    expect(send.title).toBe("대기열에 추가 (Enter)");
    await act(async () => {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true }));
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", keyCode: 229, bubbles: true }));
    });
    expect(onSend).not.toHaveBeenCalled();
    await act(async () => {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onSend).toHaveBeenCalledWith(expect.stringContaining("다음 요청"), ["/tmp/editor.png"]);
    expect(getCaptures(7)).toEqual([]);
    await act(async () => send.click());
    expect(onSend).toHaveBeenCalledTimes(2);
  });
});


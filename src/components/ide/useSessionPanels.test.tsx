// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useSessionPanels, type SessionPanels } from "./useSessionPanels";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const A = "local:1";
const B = "local:2";
const C = "local:3";

let container: HTMLDivElement | null = null;
let root: Root | null = null;
/** 마지막 렌더가 본 패널 상태 — 화면에 그려지는 값이다. */
let panels: SessionPanels | null = null;

function Probe({ sessionKey }: { sessionKey: string | null }) {
  panels = useSessionPanels(sessionKey);
  return <div data-open={panels.codeOpen} />;
}

const open = async (sessionKey: string | null): Promise<void> => {
  await act(async () => {
    root?.render(<Probe sessionKey={sessionKey} />);
  });
};

/** 사용자의 조작 — 훅이 준 setter를 그대로 부른다. */
const use = async (fn: (p: SessionPanels) => void): Promise<void> => {
  await act(async () => {
    if (panels != null) fn(panels);
  });
};

beforeEach(() => {
  localStorage.clear();
  panels = null;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  localStorage.clear();
});

describe("useSessionPanels", () => {
  it("터미널을 연 세션을 떠나면 다음 세션에는 따라오지 않는다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock(true));

    await open(B);

    expect(panels?.terminalDock).toBe(false);
  });

  it("돌아오면 그 세션에서 열어 둔 모습 그대로다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock(true));
    await use((p) => p.setCodeTab("preview"));
    await use((p) => p.setCodeOpen(true));

    await open(B);
    await open(A);

    expect(panels?.terminalDock).toBe(true);
    expect(panels?.codeTab).toBe("preview");
    expect(panels?.codeOpen).toBe(true);
  });

  it("옆 세션에서 방금 한 일이 처음 여는 세션의 시작값을 바꾸지 않는다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock(true));
    await use((p) => p.setShowTree(true));

    // B는 A와 무관하게 처음 열리는 세션이다 — seed는 이번 실행 내내 고정이다.
    await open(B);
    expect(panels?.terminalDock).toBe(false);
    expect(panels?.showTree).toBe(false);

    // A를 한 번 더 만졌다고 해서 그 뒤에 처음 여는 C가 달라지지도 않는다.
    await open(A);
    await use((p) => p.setChannelPinned(false));
    await open(C);
    expect(panels?.terminalDock).toBe(false);
    expect(panels?.channelPinned).toBe(true);
  });

  it("각 세션이 자기 패널 구성을 따로 들고 있다", async () => {
    await open(A);
    await use((p) => p.setCodeOpen(true));
    await use((p) => p.setCodeTab("preview"));

    await open(B);
    await use((p) => p.setTerminalDock(true));

    await open(A);
    expect(panels?.codeOpen).toBe(true);
    expect(panels?.codeTab).toBe("preview");
    expect(panels?.terminalDock).toBe(false);

    await open(B);
    expect(panels?.terminalDock).toBe(true);
    expect(panels?.codeOpen).toBe(false);
  });

  it("id가 같아도 호스트가 다르면 서로 다른 세션이다", async () => {
    await open("local:3");
    await use((p) => p.setTerminalDock(true));

    await open("box:3");

    expect(panels?.terminalDock).toBe(false);
  });

  it("처음 여는 세션은 저장된 코드 열 상태와 무관하게 플로팅 채널로 시작한다", async () => {
    localStorage.setItem("praxis-workspace-code-open", "true");

    await open(A);

    expect(panels?.codeOpen).toBe(false);
    expect(panels?.channelPinned).toBe(true);
  });

  it("코드 열을 여닫아도 새 세션의 시작값으로 지속하지 않는다", async () => {
    await open(A);
    await use((p) => p.setCodeOpen(true));

    expect(localStorage.getItem("praxis-workspace-code-open")).toBeNull();
  });

  it("터미널 도크는 지속되지 않는다 — 다음 실행은 닫힌 채로 시작한다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock(true));

    expect(localStorage.getItem("praxis-workspace-terminal-dock")).toBeNull();
  });

  it("홈(선택 없음)으로 나갔다 돌아와도 그 세션의 구성이 남는다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock(true));

    await open(null);
    expect(panels?.terminalDock).toBe(false);

    await open(A);
    expect(panels?.terminalDock).toBe(true);
  });

  it("함수형 갱신으로 토글할 수 있다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock((v) => !v));
    expect(panels?.terminalDock).toBe(true);

    await use((p) => p.setTerminalDock((v) => !v));
    expect(panels?.terminalDock).toBe(false);
  });

  it("상한을 넘으면 가장 오래 손대지 않은 세션부터 버린다", async () => {
    await open(A);
    await use((p) => p.setTerminalDock(true));

    // A 이후 서로 다른 세션 50개를 만진다 — A가 상한 밖으로 밀린다.
    for (let i = 0; i < 50; i += 1) {
      await open(`local:${100 + i}`);
      await use((p) => p.setTerminalDock(true));
    }

    await open(A);
    expect(panels?.terminalDock).toBe(false);
  });

  it("트리 렌즈를 host:id별로 복원한다 — 변경을 보던 세션만 변경으로 돌아온다", async () => {
    await open(A);
    await use((p) => p.openChangesLens());
    expect(panels?.showTree).toBe(true);
    expect(panels?.treeLens).toBe("changes");

    await open(B);
    expect(panels?.showTree).toBe(false);
    expect(panels?.treeLens).toBe("files");

    await open(A);
    expect(panels?.showTree).toBe(true);
    expect(panels?.treeLens).toBe("changes");
  });

  it("닫힌 트리의 렌즈는 남는다 — 변경을 보다 닫고 다시 열면 변경이 나온다", async () => {
    await open(A);
    await use((p) => p.openChangesLens());
    await use((p) => p.setShowTree(false));
    expect(panels?.treeLens).toBe("changes");

    await use((p) => p.setShowTree(true));
    expect(panels?.treeLens).toBe("changes");
  });

  it("openChangesLens는 멱등이다 — 이미 그 상태면 코드 열을 건드리지 않는다", async () => {
    await open(A);
    await use((p) => p.setCodeOpen(true));
    await use((p) => p.setCodeTab("preview"));
    await use((p) => p.openChangesLens());
    await use((p) => p.openChangesLens());

    expect(panels?.showTree).toBe(true);
    expect(panels?.treeLens).toBe("changes");
    expect(panels?.codeOpen).toBe(true);
    expect(panels?.codeTab).toBe("preview");
  });
});

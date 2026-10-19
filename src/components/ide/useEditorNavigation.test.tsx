// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useEditorNavigation, type EditorNavigationIdentity } from "./useEditorNavigation";

(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true;

let api: ReturnType<typeof useEditorNavigation> | null = null;
const target = {
  path: "src/b.ts",
  abs_path: "/w/src/b.ts",
  line: 8,
  column: 2,
  external: false,
};
const origin = {
  path: "src/a.ts",
  groupId: "g1",
  line: 3,
  column: 1,
  scrollTop: 0,
  scrollLeft: 0,
};

function Harness({
  identity = { host: "local", taskId: 1, windowId: "main" },
}: { identity?: EditorNavigationIdentity }) {
  api = useEditorNavigation({
    identity,
    openTarget: async () => "opened",
    onError: () => undefined,
  });
  return null;
}

describe("useEditorNavigation", () => {
  let root: Root;
  let node: HTMLDivElement;
  beforeEach(() => {
    node = document.createElement("div");
    document.body.append(node);
    root = createRoot(node);
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    node.remove();
    api = null;
    vi.useRealTimers();
  });

  it("commits history only after reveal acknowledgement", async () => {
    await act(async () => root.render(<Harness />));
    await act(async () => {
      await api?.navigate(target, origin);
    });
    expect(api?.canBack).toBe(false);
    await act(async () => api?.onRevealed(api?.reveal ?? undefined));
    expect(api?.canBack).toBe(true);
  });

  it("accepts only the newest concurrent open", async () => {
    let resolveFirst: ((result: "opened") => void) | undefined;
    let calls = 0;
    function ConcurrentHarness() {
      api = useEditorNavigation({
        identity: { host: "local", taskId: 1, windowId: "main" },
        openTarget: () => {
          calls += 1;
          return calls === 1
            ? new Promise((resolve) => {
                resolveFirst = resolve;
              })
            : Promise.resolve("opened");
        },
        onError: () => undefined,
      });
      return null;
    }
    await act(async () => root.render(<ConcurrentHarness />));
    await act(async () => {
      api?.navigate(target, origin);
      api?.navigate({ ...target, path: "src/c.ts" }, origin);
    });
    expect(api?.reveal?.path).toBe("src/c.ts");
    await act(async () => resolveFirst?.("opened"));
    expect(api?.reveal?.path).toBe("src/c.ts");
  });

  it("clears the old reveal and ignores its acknowledgement on a newer request", async () => {
    let resolveSecond: ((result: "opened") => void) | undefined;
    let calls = 0;
    function PendingHarness() {
      api = useEditorNavigation({
        identity: { host: "local", taskId: 1, windowId: "main" },
        openTarget: () => {
          calls += 1;
          return calls === 1
            ? Promise.resolve("opened" as const)
            : new Promise((resolve) => {
                resolveSecond = resolve;
              });
        },
        onError: () => undefined,
      });
      return null;
    }
    await act(async () => root.render(<PendingHarness />));
    await act(async () => api?.navigate(target, origin));
    const firstAck = api?.reveal;
    await act(async () =>
      api?.navigate({ ...target, path: "src/c.ts" }, origin),
    );
    expect(api?.reveal).toBeNull();
    await act(async () => api?.onRevealed(firstAck ?? undefined));
    expect(api?.canBack).toBe(false);
    await act(async () => resolveSecond?.("opened"));
    await act(async () => api?.onRevealed(api?.reveal ?? undefined));
    expect(api?.canBack).toBe(true);
  });

  it("forwards an external target without committing navigation history", async () => {
    const openTarget = vi.fn().mockResolvedValue("external");
    function ExternalHarness() {
      api = useEditorNavigation({
        identity: { host: "local", taskId: 1, windowId: "main" },
        openTarget,
        onError: () => undefined,
      });
      return null;
    }
    await act(async () => root.render(<ExternalHarness />));
    const external = { ...target, path: null, external: true };
    await act(async () => api?.navigate(external, origin));

    expect(openTarget).toHaveBeenCalledWith(external);
    expect(api?.canBack).toBe(false);
  });

  it("reports a current open rejection", async () => {
    const onError = vi.fn();
    function RejectionHarness() {
      api = useEditorNavigation({
        identity: { host: "local", taskId: 1, windowId: "main" },
        openTarget: async () => Promise.reject(new Error("open failed")),
        onError,
      });
      return null;
    }
    await act(async () => root.render(<RejectionHarness />));
    await act(async () => api?.navigate(target, origin));
    expect(onError).toHaveBeenCalledWith("파일을 열지 못했습니다");
    expect(api?.canBack).toBe(false);
  });

  it("skips a missing back entry and reveals the next available location", async () => {
    let missing = false;
    function MissingEntryHarness() {
      api = useEditorNavigation({
        identity: { host: "local", taskId: 1, windowId: "main" },
        openTarget: async (next) =>
          missing && next.path === "src/b.ts" ? "failed" : "opened",
        onError: () => undefined,
      });
      return null;
    }
    await act(async () => root.render(<MissingEntryHarness />));
    await act(async () => api?.navigate(target, origin));
    await act(async () => api?.onRevealed(api?.reveal ?? undefined));
    await act(async () =>
      api?.navigate(
        { ...target, path: "src/c.ts" },
        { ...origin, path: "src/b.ts" },
      ),
    );
    await act(async () => api?.onRevealed(api?.reveal ?? undefined));

    missing = true;
    await act(async () => api?.back());

    expect(api?.reveal).toMatchObject({ path: "src/a.ts", restore: true });
  });

  it("discards a pending navigation when identity changes", async () => {
    await act(async () => root.render(<Harness />));
    await act(async () => {
      await api?.navigate(target, origin);
    });
    await act(async () =>
      root.render(
        <Harness identity={{ host: "remote", taskId: 1, windowId: "main" }} />,
      ),
    );
    await act(async () => api?.onRevealed(api?.reveal ?? undefined));
    expect(api?.canBack).toBe(false);
  });

  it("supports project navigation and discards a reveal when its root changes", async () => {
    await act(async () => root.render(<Harness identity={{ host: "local", taskId: null, windowId: "editor", workspaceKey: "/repo/one" }} />));
    await act(async () => { await api?.navigate(target, origin); });
    expect(api?.reveal?.path).toBe(target.path);
    await act(async () => root.render(<Harness identity={{ host: "local", taskId: null, windowId: "editor", workspaceKey: "/repo/two" }} />));
    expect(api?.reveal).toBeNull();
    expect(api?.canBack).toBe(false);
  });

  it("expires an unacknowledged reveal after five seconds", async () => {
    vi.useFakeTimers();
    const onError = vi.fn();
    function TimeoutHarness() {
      api = useEditorNavigation({
        identity: { host: "local", taskId: 1, windowId: "main" },
        openTarget: async () => "opened",
        onError,
      });
      return null;
    }
    await act(async () => root.render(<TimeoutHarness />));
    await act(async () => {
      await api?.navigate(target, origin);
    });
    await act(async () => vi.advanceTimersByTimeAsync(5_000));
    expect(onError).toHaveBeenCalledOnce();
    expect(api?.canBack).toBe(false);
  });
});

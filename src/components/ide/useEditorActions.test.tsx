// @vitest-environment jsdom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const permissionError = new Error("Not allowed to open path /w/docs/설계 문서/한글 파일.md");
const mocks = vi.hoisted(() => ({
  openPath: vi.fn(async () => { throw permissionError; }),
  openLocalFile: vi.fn(async () => undefined),
  resolveAbsPath: vi.fn(async (_id: number, path: string) => `/w/${path}`),
  lspGoto: vi.fn(async () => []),
  lspStatus: vi.fn(async () => ({ available: true, server: "jdtls", detail: null, phase: "idle" })),
  semanticTokens: vi.fn(async () => null),
  projectGoto: vi.fn(async () => []),
  projectStatus: vi.fn(async () => ({ available: true, server: "jdtls", detail: null, phase: "analyzing" })),
  projectTokens: vi.fn(async () => null),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: mocks.openPath,
  revealItemInDir: vi.fn(),
}));
vi.mock("../../lib/ipc", () => ({
  lspGoto: mocks.lspGoto,
  lspStatus: mocks.lspStatus,
  lspSemanticTokens: mocks.semanticTokens,
  openLocalFile: mocks.openLocalFile,
  resolveAbsPath: mocks.resolveAbsPath,
}));
vi.mock("../../lib/project-editor-ipc", () => ({
  projectEditorLspGoto: mocks.projectGoto,
  projectEditorLspStatus: mocks.projectStatus,
  projectEditorLspSemanticTokens: mocks.projectTokens,
}));
vi.mock("../../lib/transport", () => ({
  LOCAL_HOST: "local",
  getTransport: (host: string) => ({ kind: host === "local" ? "local" : "remote" }),
}));

import { useEditorActions, type EditorActions } from "./useEditorActions";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const DOCUMENT_PATH = "docs/설계 문서/한글 파일.md";
const LOCAL_42 = { host: "local", id: 42 };

let api: EditorActions | null = null;
let container: HTMLDivElement | null = null;
let root: Root | null = null;

function Harness({
  host,
  taskId = 42,
  projectRoot = null,
  onError,
  openFile = vi.fn(async () => true),
}: {
  host: string;
  taskId?: number | null;
  projectRoot?: string | null;
  onError: (message: string) => void;
  openFile?: (path: string) => Promise<boolean>;
}) {
  api = useEditorActions({ taskId, projectRoot, host, onError, openFile });
  return null;
}

const render = async (props: Parameters<typeof Harness>[0]) => {
  await act(async () => {
    root?.render(<Harness {...props} />);
    await Promise.resolve();
  });
};

const openExternal = async (path: string) => {
  await act(async () => {
    await api?.openPathExternal(path);
  });
};

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root?.unmount());
  container?.remove();
  container = null;
  root = null;
  api = null;
  vi.clearAllMocks();
});

describe("useEditorActions", () => {
  const query = { kind: "references" as const, path: "src/FepClient.java", text: "class FepClient {}", line: 1, column: 7 };

  it("프로젝트 에디터의 탐색·상태·강조는 작업 ID 없이 지정 루트로 조회한다", async () => {
    await render({ host: "local", taskId: null, projectRoot: "/repo/mirae", onError: vi.fn() });
    await api?.gotoSymbol(query);
    await api?.askLspStatus(query.path);
    await api?.askSemanticTokens(query.path, query.text);
    expect(mocks.projectGoto).toHaveBeenCalledWith("/repo/mirae", query.path, query.text, 1, 7, "references");
    expect(mocks.projectStatus).toHaveBeenCalledWith("/repo/mirae", query.path);
    expect(mocks.projectTokens).toHaveBeenCalledWith("/repo/mirae", query.path, query.text);
    expect(mocks.lspGoto).not.toHaveBeenCalled();
  });

  it("루트 전환 뒤 같은 콜백은 현재 프로젝트를 조회한다", async () => {
    await render({ host: "local", taskId: null, projectRoot: "/repo/one", onError: vi.fn() });
    const goto = api!.gotoSymbol;
    await render({ host: "local", taskId: null, projectRoot: "/repo/two", onError: vi.fn() });
    await goto(query);
    expect(mocks.projectGoto).toHaveBeenCalledWith("/repo/two", query.path, query.text, 1, 7, "references");
  });

  it("원격 작업에서는 로컬 분석 서버를 시작하지 않는다", async () => {
    await render({ host: "remote", onError: vi.fn() });
    await expect(api?.gotoSymbol(query)).resolves.toEqual([]);
    await expect(api?.askSemanticTokens(query.path, query.text)).resolves.toBeNull();
    expect((await api?.askLspStatus(query.path))?.available).toBe(false);
    expect(mocks.lspGoto).not.toHaveBeenCalled();
    expect(mocks.semanticTokens).not.toHaveBeenCalled();
  });

  it("탭 열기 중 프로젝트가 바뀌면 이전 이동을 폐기한다", async () => {
    let finish!: (opened: boolean) => void;
    const openFile = vi.fn(() => new Promise<boolean>((resolve) => { finish = resolve; }));
    await render({ host: "local", taskId: null, projectRoot: "/repo/one", onError: vi.fn(), openFile });
    const pending = api?.openLspTarget({ path: query.path, abs_path: `/repo/one/${query.path}`, line: 1, column: 1, external: false });
    await render({ host: "local", taskId: null, projectRoot: "/repo/two", onError: vi.fn(), openFile });
    finish(true);
    await expect(pending).resolves.toBe("failed");
  });
  it("로컬 한글·공백 문서는 opener 플러그인 대신 코어 IPC로 연다", async () => {
    const onError = vi.fn();
    await render({ host: "local", onError });

    await openExternal(DOCUMENT_PATH);

    expect(onError).not.toHaveBeenCalled();
    expect(mocks.openLocalFile).toHaveBeenCalledWith(LOCAL_42, DOCUMENT_PATH);
    expect(mocks.openPath).not.toHaveBeenCalled();
  });

  it("외부 LSP 대상도 opener 플러그인 대신 코어 IPC로 연다", async () => {
    const onError = vi.fn();
    await render({ host: "local", onError });

    let outcome;
    await act(async () => {
      outcome = await api?.openLspTarget({
        path: null,
        abs_path: `/w/${DOCUMENT_PATH}`,
        line: 1,
        column: 1,
        external: true,
      });
    });

    expect(onError).not.toHaveBeenCalled();
    expect(outcome).toBe("external");
    expect(mocks.openLocalFile).toHaveBeenCalledWith(LOCAL_42, `/w/${DOCUMENT_PATH}`);
    expect(mocks.openPath).not.toHaveBeenCalled();
  });

  it("원격 작업은 IPC를 부르지 않고 열기 불가 오류를 보인다", async () => {
    const onError = vi.fn();
    await render({ host: "remote", onError });

    await openExternal(DOCUMENT_PATH);

    expect(onError).toHaveBeenCalledWith("원격 작업에서는 열 수 없는 위치입니다");
    expect(mocks.openLocalFile).not.toHaveBeenCalled();
    expect(mocks.openPath).not.toHaveBeenCalled();
  });

  it("작업 전환 뒤 현재 원격 호스트를 기준으로 막는다", async () => {
    const onError = vi.fn();
    await render({ host: "local", onError });
    await render({ host: "remote", taskId: 43, onError });

    await openExternal(DOCUMENT_PATH);

    expect(onError).toHaveBeenCalledWith("원격 작업에서는 열 수 없는 위치입니다");
    expect(mocks.openLocalFile).not.toHaveBeenCalled();
  });

  it("작업이 없으면 외부 LSP 대상을 열지 않는다", async () => {
    const onError = vi.fn();
    await render({ host: "local", taskId: null, onError });

    let outcome;
    await act(async () => {
      outcome = await api?.openLspTarget({ path: null, abs_path: `/w/${DOCUMENT_PATH}`, line: 1, column: 1, external: true });
    });

    expect(outcome).toBe("failed");
    expect(mocks.openLocalFile).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it("코어 IPC 오류의 알려진 기계 접두사를 숨긴다", async () => {
    const onError = vi.fn();
    mocks.openLocalFile.mockRejectedValueOnce(new Error("차단됨"));
    await render({ host: "local", onError });

    await openExternal(DOCUMENT_PATH);

    expect(onError).toHaveBeenCalledWith("Error: 차단됨");
  });

  it("원격 외부 LSP 대상은 실패로 끝나고 IPC를 부르지 않는다", async () => {
    const onError = vi.fn();
    await render({ host: "remote", onError });
    const outcome = await api?.openLspTarget({ path: null, abs_path: `/w/${DOCUMENT_PATH}`, line: 1, column: 1, external: true });
    expect(outcome).toBe("failed");
    expect(mocks.openLocalFile).not.toHaveBeenCalled();
  });

  it("내부 LSP 대상은 탭만 열고 reveal은 컨트롤러에 맡긴다", async () => {
    const onError = vi.fn();
    const openFile = vi.fn(async () => true);
    await render({ host: "local", onError, openFile });
    const outcome = await api?.openLspTarget({ path: DOCUMENT_PATH, abs_path: `/w/${DOCUMENT_PATH}`, line: 3, column: 2, external: false });
    expect(outcome).toBe("opened");
    expect(openFile).toHaveBeenCalledWith(DOCUMENT_PATH);
    expect(api?.revealTarget).toBeNull();
  });

  it("내부 LSP 열기 중 작업이나 호스트가 바뀌면 실패한다", async () => {
    const onError = vi.fn();
    let finishOpen: (opened: boolean) => void = () => undefined;
    const openFile = vi.fn(() => new Promise<boolean>((resolve) => { finishOpen = resolve; }));
    await render({ host: "local", onError, openFile });
    const outcome = api?.openLspTarget({ path: DOCUMENT_PATH, abs_path: `/w/${DOCUMENT_PATH}`, line: 3, column: 2, external: false });
    await render({ host: "remote", taskId: 43, onError, openFile });
    finishOpen(true);

    await expect(outcome).resolves.toBe("failed");
    expect(api?.revealTarget).toBeNull();
  });
});

import { useCallback, useRef, useState } from "react";
import type { HostId } from "../../lib/transport";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  lspGoto,
  lspStatus,
  openLocalFile,
  resolveAbsPath,
  type LspGotoKind,
  type LspStatusInfo,
  type LspTarget,
} from "../../lib/ipc";
import { getTransport } from "../../lib/transport";

/** ⌘B 착지 지점 — EditorPane이 대상 탭을 띄운 뒤 소비하고 null로 되돌린다. */
export interface RevealTarget {
  path: string;
  line: number;
  column: number;
}

export type OpenTargetOutcome = "opened" | "external" | "failed";

interface Options {
  taskId: number | null;
  /** 작업이 사는 호스트 — 언어 서버는 로컬에서만 뜬다. */
  host: HostId;
  onError: (message: string) => void;
  /** 이동 대상이 워크트리 안 파일일 때 탭을 여는 쪽. `useWorkspaceFiles.openFile`을 넘긴다.
   *  반환값은 실제로 열렸는지 — 열지 못한 탭에 착지 지점을 걸지 않기 위해 본다. */
  openFile: (path: string) => Promise<boolean>;
}

export interface EditorActions {
  revealTarget: RevealTarget | null;
  setRevealTarget: (target: RevealTarget | null) => void;
  /** `path:line` 표기로 온 착지 지점을 건다. 줄이 없거나 1보다 작으면 커서를 옮기지 않는다.
   *  두 창이 같은 규칙을 각자 구현하면(기본 열 1, 0줄 처리) 한쪽만 고쳐졌을 때 조용히 갈라진다. */
  revealAt: (
    path: string,
    line: number | null | undefined,
    column: number | null | undefined,
  ) => void;
  openPathExternal: (path: string) => Promise<void>;
  revealPathInFinder: (path: string) => Promise<void>;
  /** 워크트리 절대 경로를 클립보드로 — 탭 메뉴의 "절대 경로 복사". 저장된 경로는 워크트리
   *  기준 상대라, 터미널이나 다른 앱에 붙이려면 루트를 붙여야 쓸모가 있다. */
  copyAbsPath: (path: string) => Promise<void>;
  gotoSymbol: (req: {
    kind: LspGotoKind;
    path: string;
    text: string;
    line: number;
    column: number;
  }) => Promise<LspTarget[]>;
  askLspStatus: (path: string) => Promise<LspStatusInfo>;
  openLspTarget: (target: LspTarget) => Promise<OpenTargetOutcome>;
}

/**
 * 에디터가 파일 내용 밖에서 하는 일들 — OS로 넘기기, Finder에서 보기, 심볼 이동.
 *
 * `useWorkspaceFiles`(열린 탭·내용)와 갈라 둔 이유는 책임이 다르기 때문이다. 이쪽은 상태를
 * 거의 갖지 않고 IPC로 나가며, 원격 연결에서는 통째로 비활성이 된다. 메인 창과 에디터 창이
 * 같은 것을 쓰도록 여기 모아 둔다.
 */
export function useEditorActions({ taskId, host, onError, openFile }: Options): EditorActions {
  const [revealTarget, setRevealTarget] = useState<RevealTarget | null>(null);

  const taskIdRef = useRef(taskId);
  taskIdRef.current = taskId;
  const scopeRef = useRef({ host, taskId });
  scopeRef.current = { host, taskId };
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const openFileRef = useRef(openFile);
  openFileRef.current = openFile;

  const openPathExternal = useCallback(async (path: string) => {
    const scope = scopeRef.current;
    if (scope.taskId == null) return;
    if (getTransport(scope.host).kind !== "local") {
      onErrorRef.current("원격 작업에서는 열 수 없는 위치입니다");
      return;
    }
    try {
      await openLocalFile({ host: scope.host, id: scope.taskId }, path);
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }, []);

  // Finder에서 보기 — 파일이 선택된 채 폴더를 연다.
  const revealPathInFinder = useCallback(async (path: string) => {
    const id = taskIdRef.current;
    if (id == null) return;
    try {
      await revealItemInDir(await resolveAbsPath(id, path));
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }, []);

  const copyAbsPath = useCallback(async (path: string) => {
    const id = taskIdRef.current;
    if (id == null) return;
    try {
      await navigator.clipboard.writeText(await resolveAbsPath(id, path));
    } catch (e) {
      onErrorRef.current(String(e));
    }
  }, []);

  // ⌘B 계열 — 언어 서버는 워크트리 파일시스템을 직접 읽으므로 로컬 작업에서만 붙는다.
  const gotoSymbol = useCallback(
    async (req: {
      kind: LspGotoKind;
      path: string;
      text: string;
      line: number;
      column: number;
    }) => {
      const id = taskIdRef.current;
      if (id == null) return [];
      return lspGoto(id, req.path, req.text, req.line, req.column, req.kind);
    },
    [],
  );

  /** 이 파일에서 정의 이동이 되는지 — 에디터의 상태 배지가 파일을 바꿀 때마다 물어본다.
   *  참조가 매 렌더 바뀌면 그 조회가 렌더마다 다시 도므로 고정해 둔다. */
  const askLspStatus = useCallback(async (path: string) => {
    const id = taskIdRef.current;
    if (id == null) return { available: false, server: null, detail: null };
    return lspStatus(id, path);
  }, []);

  const revealAt = useCallback(
    (path: string, line: number | null | undefined, column: number | null | undefined) => {
      // `docs/a.md:0`도 링크 문법을 통과한다(`agent-link.ts`의 LINE_SUFFIX). 0줄은 없다.
      if (line == null || line < 1) return;
      setRevealTarget({ path, line, column: column != null && column > 0 ? column : 1 });
    },
    [],
  );

  /** 이동 대상으로 착지 — 탭을 열고(이미 열려 있으면 활성화) 그 줄로 커서를 옮긴다.
   *  워크트리 밖(의존성·표준 라이브러리)은 편집 탭으로 열지 않고 기본 앱에 넘긴다. */
  const openLspTarget = useCallback(async (target: LspTarget) => {
    const scope = scopeRef.current;
    if (target.external || target.path == null) {
      if (scope.taskId == null) return "failed";
      if (getTransport(scope.host).kind !== "local") {
        onErrorRef.current("원격 작업에서는 열 수 없는 위치입니다");
        return "failed";
      }
      try {
        await openLocalFile({ host: scope.host, id: scope.taskId }, target.abs_path);
        return "external";
      } catch (e) {
        onErrorRef.current(String(e));
        return "failed";
      }
    }
    if (!(await openFileRef.current(target.path))) return "failed";
    if (
      scope.host !== scopeRef.current.host ||
      scope.taskId !== scopeRef.current.taskId
    )
      return "failed";
    return "opened";
  }, []);

  return {
    revealTarget,
    setRevealTarget,
    revealAt,
    openPathExternal,
    revealPathInFinder,
    copyAbsPath,
    gotoSymbol,
    askLspStatus,
    openLspTarget,
  };
}

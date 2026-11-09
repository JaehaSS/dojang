import { useCallback, useMemo, useState, type ReactElement } from "react";
import { ShellTerminal, type PtyChannel } from "./ShellTerminal";
import { Icon } from "./icons";
import { IconButton } from "../ui";
import { useTerminalDockHeight } from "./terminal-dock-height";
import { replDetach, replOpen, replReplay, replResize, replWrite } from "../../lib/ipc";

interface Props {
  taskId: number;
  /** 콘솔을 띄울 수 있는 작업인지 — 원격 작업은 워크트리가 로컬에 없어 열 수 없다. */
  available: boolean;
  /** 헤더에 띄울 작업 위치(워크트리 이름). */
  label?: string;
  codeFontFamily?: string;
  codeFontSize?: number;
  /** 콘솔 세션이 열려 스트림이 붙었다 — 열리기를 기다리던 코드를 흘려보낼 시점. */
  onOpened?: () => void;
  onClose: () => void;
}

/** ipython이 없을 때의 도크 상태. `python`이 null이면 설치할 인터프리터조차 없다. */
type Missing = { python: string | null };

/** 에디터 팝아웃 아래에 붙는 Python 콘솔(IPython) 도크 — 워크스페이스 셸과 별개의 PTY.
 *
 *  ipython이 없으면 백엔드가 띄우지 않고 `missing`을 돌려주므로, 그 자리에 설치 제안을 띄운다.
 *  설치를 고르면 `install: true`로 다시 열고(pip → 그 자리에서 IPython 기동) 터미널을 다시
 *  마운트한다 — 설치 진행 출력도 같은 PTY로 흘러 사용자가 본다. */
export function ReplDock({
  taskId,
  available,
  label,
  codeFontFamily,
  codeFontSize,
  onOpened,
  onClose,
}: Props): ReactElement {
  const resize = useTerminalDockHeight();
  const [missing, setMissing] = useState<Missing | null>(null);
  /** 설치를 승인한 시도 번호 — 바뀌면 터미널을 다시 마운트해 `install: true`로 연다. */
  const [attempt, setAttempt] = useState(0);
  const [install, setInstall] = useState(false);

  const channel = useMemo<PtyChannel>(
    () => ({
      prefix: "repl",
      open: async (id, cols, rows) => {
        const result = await replOpen(id, cols, rows, install);
        if (result.status === "missing") {
          setMissing({ python: result.python });
          throw new Error("ipython이 없습니다");
        }
        return result.status === "existed";
      },
      write: replWrite,
      resize: replResize,
      replay: replReplay,
      detach: replDetach,
    }),
    [install],
  );

  const approveInstall = useCallback(() => {
    setMissing(null);
    setInstall(true);
    setAttempt((n) => n + 1);
  }, []);

  return (
    <section
      className="relative shrink-0 border-t border-border bg-surface flex flex-col min-h-0"
      style={{ height: resize.height }}
      aria-label="Python 콘솔"
    >
      <div
        role="separator"
        aria-label="콘솔 높이 조절"
        aria-orientation="horizontal"
        tabIndex={0}
        className="absolute inset-x-0 top-0 z-20 h-1 cursor-row-resize hover:bg-primary/50 focus:bg-primary/50"
        onPointerDown={resize.onPointerDown}
        onKeyDown={resize.onKeyDown}
      />
      <div className="h-8 shrink-0 flex items-center gap-2 px-3 border-b border-border">
        <Icon name="terminal" size={14} />
        <span className="text-xs uppercase tracking-wide text-text-muted">Python 콘솔</span>
        {label != null && (
          <span className="text-xs text-text-muted truncate opacity-70">{label}</span>
        )}
        <span className="ml-auto text-[11px] text-text-muted hidden sm:inline">⇧⏎ 선택 실행</span>
        <IconButton size="sm" label="콘솔 닫기" title="콘솔 닫기 (⌃`)" onClick={onClose}>
          <Icon name="x" size={14} />
        </IconButton>
      </div>
      {!available ? (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-xs text-text-muted">
          Python 콘솔은 로컬 작업에서만 사용할 수 있습니다.
        </div>
      ) : missing ? (
        <div
          className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-sm text-text-secondary"
          role="status"
        >
          <div>
            이 워크트리에서 <span className="font-mono">ipython</span>을 찾지 못했습니다.
            <br />
            <span className="text-xs text-text-muted">
              찾는 순서: $VIRTUAL_ENV → .venv → venv → 로그인 셸 PATH
            </span>
          </div>
          {missing.python ? (
            <button
              className="h-8 px-3 rounded-md bg-primary text-bg text-sm font-medium hover:opacity-90 flex items-center gap-1.5"
              onClick={approveInstall}
            >
              <Icon name="play" size={14} />
              {`${missing.python} -m pip install ipython`}
            </button>
          ) : (
            <div className="text-xs text-danger" role="alert">
              python3도 없어 설치할 수 없습니다. 먼저 Python을 설치하거나 가상환경을 만드세요.
            </div>
          )}
        </div>
      ) : (
        <ShellTerminal
          taskId={taskId}
          autoFocus
          channel={channel}
          onOpened={onOpened}
          codeFontFamily={codeFontFamily}
          codeFontSize={codeFontSize}
          key={`dock-repl-${taskId}-${attempt}`}
        />
      )}
    </section>
  );
}

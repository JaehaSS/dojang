import type { ReactElement } from "react";
import { ShellTerminal } from "./ShellTerminal";
import { Icon } from "./icons";
import { IconButton } from "../ui";
import { useTerminalDockHeight } from "./terminal-dock-height";

interface Props {
  taskId: number;
  /** 셸을 띄울 수 있는 작업인지 — 원격 작업은 워크트리가 로컬에 없어 열 수 없다. */
  available: boolean;
  /** 헤더에 띄울 작업 위치(워크트리 이름) — 어느 디렉터리의 셸인지 보이게 한다. */
  label?: string;
  codeFontFamily?: string;
  codeFontSize?: number;
  onClose: () => void;
}

/** 에디터 아래에 붙는 워크스페이스 셸 도크 — 작업 워크트리에서 도는 인터랙티브 셸(로컬 전용).
 *  우측 패널의 터미널 탭과 같은 백엔드 셸(작업당 1개)을 쓴다. 두 곳에 동시에 띄우면
 *  같은 PTY를 두 xterm이 서로 다른 크기로 리사이즈하므로, App에서 도크를 우선한다. */
export function TerminalDock({
  taskId,
  available,
  label,
  codeFontFamily,
  codeFontSize,
  onClose,
}: Props): ReactElement {
  const resize = useTerminalDockHeight();
  return (
    <section
      className="relative shrink-0 border-t border-border bg-surface flex flex-col min-h-0"
      style={{ height: resize.height }}
      aria-label="터미널"
    >
      <div
        role="separator"
        aria-label="터미널 높이 조절"
        aria-orientation="horizontal"
        tabIndex={0}
        className="absolute inset-x-0 top-0 z-20 h-1 cursor-row-resize hover:bg-primary/50 focus:bg-primary/50"
        onPointerDown={resize.onPointerDown}
        onKeyDown={resize.onKeyDown}
      />
      <div className="h-8 shrink-0 flex items-center gap-2 px-3 border-b border-border">
        <Icon name="terminal" size={14} />
        <span className="text-xs uppercase tracking-wide text-text-muted">터미널</span>
        {label != null && (
          <span className="text-xs text-text-muted truncate opacity-70">{label}</span>
        )}
        <IconButton size="sm" className="ml-auto" label="터미널 닫기" title="터미널 닫기 (⌃`)" onClick={onClose}>
          <Icon name="x" size={14} />
        </IconButton>
      </div>
      {available ? (
        <ShellTerminal
          taskId={taskId}
          autoFocus
          codeFontFamily={codeFontFamily}
          codeFontSize={codeFontSize}
          key={`dock-shell-${taskId}`}
        />
      ) : (
        <div className="flex flex-1 items-center justify-center px-6 text-center text-xs text-text-muted">
          터미널은 로컬 작업에서만 사용할 수 있습니다.
        </div>
      )}
    </section>
  );
}

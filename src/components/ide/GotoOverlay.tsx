import { useEffect, useRef } from "react";
import type { LspTarget } from "../../lib/ipc";
import { targetLabel } from "../../lib/lsp";
import { Icon } from "./icons";
import { IconButton } from "../ui";

/** 정의 이동 한 번의 진행 상태. 결과가 하나면 곧장 점프하므로 여기 오지 않는다. */
export type GotoState =
  | { phase: "loading"; label: string }
  | { phase: "error"; label: string; message: string }
  | { phase: "empty"; label: string }
  | { phase: "choose"; label: string; targets: LspTarget[] };

interface Props {
  state: GotoState;
  onPick: (target: LspTarget) => void;
  onClose: () => void;
}

/** 에디터 위에 겹쳐 뜨는 이동 결과 패널 — 후보가 여럿이거나 실패했을 때만 보인다.
 *
 * Peek 창처럼 코드를 미리 보여주지는 않는다. 목록에서 고르면 곧바로 탭이 열리므로
 * 한 단계를 더 끼우지 않았다. */
export function GotoOverlay({ state, onPick, onClose }: Props) {
  const boxRef = useRef<HTMLDivElement>(null);

  // 후보 목록은 키보드로 끝내야 한다 — 마우스로 옮겨가면 ⌘B의 이점이 사라진다.
  useEffect(() => {
    if (state.phase === "choose") boxRef.current?.querySelector("button")?.focus();
  }, [state]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      ref={boxRef}
      className="absolute right-3 top-3 z-20 w-96 max-h-80 flex flex-col rounded-md border border-border bg-raised shadow-e2 overflow-hidden"
      role="dialog"
      aria-label="정의 이동 결과"
    >
      <div className="h-8 flex items-center gap-2 px-3 border-b border-border shrink-0">
        <span className="text-xs text-text-secondary">{state.label}</span>
        <IconButton size="sm" className="ml-auto" label="닫기" onClick={onClose}>
          <Icon name="x" size={14} />
        </IconButton>
      </div>

      {state.phase === "loading" ? (
        <div className="px-3 py-3 text-sm text-text-muted">찾는 중…</div>
      ) : state.phase === "error" ? (
        <div className="px-3 py-3 text-sm text-status-failed whitespace-pre-wrap">
          {state.message}
        </div>
      ) : state.phase === "empty" ? (
        <div className="px-3 py-3 text-sm text-text-muted">결과가 없습니다.</div>
      ) : (
        <div className="overflow-y-auto py-1" role="list">
          {state.targets.map((t) => (
            <button
              key={`${t.abs_path}:${t.line}:${t.column}`}
              role="listitem"
              className="w-full text-left px-3 py-1.5 text-sm text-text-secondary hover:bg-bg hover:text-text focus-visible:bg-bg focus-visible:text-text flex items-center gap-2"
              onClick={() => onPick(t)}
              title={`${t.abs_path}:${t.line}`}
            >
              <span className="truncate flex-1">{targetLabel(t)}</span>
              {t.external && (
                <span className="text-xs text-text-muted shrink-0">워크트리 밖</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

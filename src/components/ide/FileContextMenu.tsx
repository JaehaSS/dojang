import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { DirEntry } from "../../lib/ipc";

export type FileMenuAction =
  | "open"
  | "terminal"
  | "reveal"
  | "copy"
  | "paste"
  | "duplicate"
  | "rename"
  | "trash"
  | "newFile"
  | "newDir"
  | "copyPath";

export interface FileMenuState {
  x: number;
  y: number;
  entry: DirEntry;
}

interface Props {
  menu: FileMenuState | null;
  /** 변경 조작을 막는 사유 — null이면 활성. 프런트 표시용 판정 결과(PRD §5.5). */
  block: { taskId: number; reason: string } | null;
  /** 원격 연결이면 변경·복제 그룹을 통째로 제거한다(PRD F-01). */
  readOnly: boolean;
  /** 내부 클립보드에 담긴 것 — 없으면 "붙여넣기" 항목 자체를 숨긴다(비활성이 아니라 제거). */
  clipboard: string | null;
  onAction: (action: FileMenuAction) => void;
  onClose: () => void;
}

const MARGIN = 8;

/** 렌더할 항목 — `null`은 구분선. */
type Row = { action: FileMenuAction; label: string; danger?: boolean; guarded?: boolean } | null;

/** 파일 트리 행의 우클릭 메뉴.
 *
 * `TaskNavigationMenu`가 좌표 메뉴의 선례지만 항목이 1~2개라 화면 경계를 다루지 않는다.
 * 여기는 항목이 12개라 창 하단에서 열면 아래가 잘리므로 뷰포트 안으로 되돌린다(계획 DR-P3). */
export function FileContextMenu({ menu, block, readOnly, clipboard, onAction, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ top: 0, left: 0 });
  const [cursor, setCursor] = useState(-1);

  const rows = useMemo<Row[]>(() => {
    if (!menu) return [];
    const head: Row[] = [
      { action: "open", label: menu.entry.is_dir ? "열기" : "미리보기" },
      { action: "terminal", label: "터미널에서 열기" },
      { action: "reveal", label: "Finder에서 보기" },
    ];
    const mutating: Row[] = readOnly
      ? []
      : [
          null,
          { action: "copy", label: "복사" },
          ...(clipboard ? ([{ action: "paste", label: "붙여넣기", guarded: true }] as Row[]) : []),
          { action: "duplicate", label: "중복", guarded: true },
          null,
          { action: "rename", label: "이름 변경…", guarded: true },
          { action: "trash", label: "휴지통으로 이동", danger: true, guarded: true },
          null,
          { action: "newFile", label: "새 파일…", guarded: true },
          { action: "newDir", label: "새 폴더…", guarded: true },
        ];
    return [...head, ...mutating, null, { action: "copyPath", label: "경로 복사" }];
  }, [menu, readOnly, clipboard]);

  /** 방향키가 짚을 수 있는 행 — 구분선과 비활성 항목은 건너뛴다. */
  const enabledIndexes = useMemo(
    () => rows.flatMap((row, i) => (row && !(row.guarded && block) ? [i] : [])),
    [rows, block],
  );

  useEffect(() => {
    if (!menu) return;
    const close = (): void => onClose();
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") return close();
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Enter") return;
      e.preventDefault();
      if (e.key === "Enter") {
        const row = cursor >= 0 ? rows[cursor] : null;
        if (row) {
          onClose();
          onAction(row.action);
        }
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : -1;
      const at = enabledIndexes.indexOf(cursor);
      const next =
        at === -1
          ? enabledIndexes[step > 0 ? 0 : enabledIndexes.length - 1]
          : enabledIndexes[(at + step + enabledIndexes.length) % enabledIndexes.length];
      setCursor(next ?? -1);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
    };
  }, [menu, onClose, onAction, rows, enabledIndexes, cursor]);

  // 메뉴가 새로 열릴 때만 커서를 되돌린다 — 방향키 이동 중에는 유지해야 한다.
  useEffect(() => {
    setCursor(-1);
  }, [menu?.entry.path, menu?.x, menu?.y]);

  // PRD §6.2 — 화면 하단·우측을 넘으면 안으로 되돌린다. 항목이 12개라 실측이 필요하다.
  useLayoutEffect(() => {
    if (!menu || !ref.current) return;
    const box = ref.current.getBoundingClientRect();
    setPos({
      top: Math.min(menu.y, Math.max(MARGIN, window.innerHeight - box.height - MARGIN)),
      left: Math.min(menu.x, Math.max(MARGIN, window.innerWidth - box.width - MARGIN)),
    });
  }, [menu, rows.length]);

  if (!menu) return null;

  return (
    <div
      ref={ref}
      className="fixed z-50 min-w-[168px] max-w-[240px] rounded-md border border-border-strong bg-raised py-1 shadow-e2"
      style={{ top: pos.top, left: pos.left }}
      onMouseDown={(e) => e.stopPropagation()}
      role="menu"
      aria-label={`${menu.entry.name} 조작`}
    >
      <div className="px-3 py-1 text-xs text-text-muted truncate" title={menu.entry.path}>
        {menu.entry.name}
      </div>
      {rows.map((row, i) =>
        row === null ? (
          <div key={`sep-${i}`} className="my-1 border-t border-border" />
        ) : (
          <button
            key={row.action}
            role="menuitem"
            aria-disabled={row.guarded === true && block !== null}
            disabled={row.guarded === true && block !== null}
            title={row.guarded && block ? block.reason : undefined}
            onMouseEnter={() => setCursor(i)}
            className={`w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm hover:bg-surface disabled:opacity-40 disabled:hover:bg-transparent disabled:cursor-not-allowed ${
              row.danger ? "text-status-failed" : "text-text-secondary"
            } ${cursor === i ? "bg-surface" : ""}`}
            onClick={() => {
              onClose();
              onAction(row.action);
            }}
          >
            {row.label}
          </button>
        ),
      )}
    </div>
  );
}

import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";

/** 메뉴 한 줄. `null`은 구분선. */
export interface MenuRow {
  key: string;
  label: string;
  disabled?: boolean;
  danger?: boolean;
  onSelect: () => void;
}

interface Props {
  /** 열 자리(화면 좌표). null이면 아무것도 그리지 않는다. */
  at: { x: number; y: number } | null;
  ariaLabel: string;
  /** 무엇에 대한 메뉴인지 — 맨 위에 흐리게 적는다. */
  header?: ReactNode;
  rows: Array<MenuRow | null>;
  onClose: () => void;
  /** 메뉴가 새로 열렸음을 알리는 값. 바뀌면 키보드 커서를 처음으로 돌린다. */
  resetKey?: string;
}

const MARGIN = 8;

/** 지금 대기 중인 삼키기 리스너 — 메뉴를 연달아 닫아도 하나만 남는다. */
let pendingSwallow: ((event: MouseEvent) => void) | null = null;

/**
 * 바깥 mousedown으로 메뉴를 닫을 때, 뒤따르는 click을 한 번 삼킨다.
 *
 * 네이티브 메뉴처럼 "닫는 클릭"은 아래 요소에 닿지 않아야 한다 — 삼키지 않으면 메뉴를 닫으려고
 * 누른 프로젝트 헤더가 접히고 작업 카드가 열린다. React는 리스너를 루트 컨테이너에 붙이므로
 * window의 capture 단계에서 멈추면 onClick까지 가지 않는다.
 *
 * 셸을 쓰지 않는 좌표 메뉴(`TaskNavigationMenu`)도 같은 규칙을 따라야 해서 내보낸다.
 */
export function swallowNextClick(): void {
  if (pendingSwallow) window.removeEventListener("click", pendingSwallow, true);
  const swallow = (event: MouseEvent): void => {
    event.stopPropagation();
    event.preventDefault();
    remove();
  };
  const remove = (): void => {
    window.removeEventListener("click", swallow, true);
    if (pendingSwallow === swallow) pendingSwallow = null;
  };
  pendingSwallow = swallow;
  window.addEventListener("click", swallow, { capture: true, once: true });
  // 우클릭·드래그처럼 click이 오지 않는 경우의 뒷정리. 브라우저는 mouseup 뒤 같은 태스크에서
  // click을 동기 디스패치하므로 이 타이머는 언제나 click 다음에 돈다.
  window.addEventListener("mouseup", () => setTimeout(remove, 0), { capture: true, once: true });
}

/**
 * 좌표에 뜨는 메뉴의 껍데기 — 위치 보정·키보드 이동·바깥 클릭 닫기.
 *
 * 같은 셸이 세 곳에 필요해졌을 때 뽑았다(탭 메뉴·파일 트리 메뉴, 그리고 앞으로 생길 것).
 * 항목이 무엇인지는 부르는 쪽이 정하고 여기는 **뜨고 지는 방식만** 안다 — 그 둘을 한
 * 컴포넌트에 두면 도메인마다 다른 조건이 한 rows 계산식에 뒤섞인다.
 *
 * `FileContextMenu`(파일 브라우저)는 아직 옮기지 않았다. 승인 게이트·내부 클립보드를 항목마다
 * 물고 있어 셸만 갈아 끼우는 것으로 끝나지 않는다.
 */
export function ContextMenuShell({ at, ariaLabel, header, rows, onClose, resetKey }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ top: 0, left: 0 });
  const [cursor, setCursor] = useState(-1);

  /** 방향키가 짚을 수 있는 행 — 구분선과 비활성 항목은 건너뛴다. */
  const enabled = useMemo(
    () => rows.flatMap((row, i) => (row && row.disabled !== true ? [i] : [])),
    [rows],
  );

  useEffect(() => {
    if (at == null) return;
    const close = (): void => onClose();
    // 삼키기 리스너는 cleanup에서 지우지 않는다 — onClose()가 `at`을 null로 만들어 cleanup이
    // click보다 먼저 돌기 때문. 리스너 자신이 click 또는 mouseup 뒤에 스스로 사라진다.
    const onDown = (): void => {
      swallowNextClick();
      onClose();
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") return close();
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp" && e.key !== "Enter") return;
      e.preventDefault();
      if (e.key === "Enter") {
        const row = cursor >= 0 ? rows[cursor] : null;
        if (row && row.disabled !== true) {
          onClose();
          row.onSelect();
        }
        return;
      }
      const step = e.key === "ArrowDown" ? 1 : -1;
      const i = enabled.indexOf(cursor);
      setCursor(
        (i === -1
          ? enabled[step > 0 ? 0 : enabled.length - 1]
          : enabled[(i + step + enabled.length) % enabled.length]) ?? -1,
      );
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
    };
  }, [at, onClose, rows, enabled, cursor]);

  // 메뉴가 새로 열릴 때만 커서를 되돌린다 — 방향키 이동 중에는 유지해야 한다.
  useEffect(() => {
    setCursor(-1);
  }, [resetKey, at?.x, at?.y]);

  // 창 아래·오른쪽을 넘으면 안으로 되돌린다. 항목 수가 조건에 따라 달라져 실측이 필요하다.
  useLayoutEffect(() => {
    if (at == null || ref.current == null) return;
    const box = ref.current.getBoundingClientRect();
    setPos({
      top: Math.min(at.y, Math.max(MARGIN, window.innerHeight - box.height - MARGIN)),
      left: Math.min(at.x, Math.max(MARGIN, window.innerWidth - box.width - MARGIN)),
    });
  }, [at, rows.length]);

  if (at == null) return null;

  return (
    <div
      ref={ref}
      className="fixed z-50 min-w-[180px] max-w-[260px] rounded-md border border-border-strong bg-raised py-1 shadow-e2"
      style={{ top: pos.top, left: pos.left }}
      onMouseDown={(e) => e.stopPropagation()}
      role="menu"
      aria-label={ariaLabel}
    >
      {header != null && (
        <div className="px-3 py-1 text-xs text-text-muted truncate">{header}</div>
      )}
      {rows.map((row, i) =>
        row === null ? (
          <div key={`sep-${i}`} className="my-1 border-t border-border" />
        ) : (
          <button
            key={row.key}
            role="menuitem"
            aria-disabled={row.disabled === true}
            disabled={row.disabled === true}
            onMouseEnter={() => setCursor(i)}
            className={`w-full text-left flex items-center gap-2 px-3 py-1.5 text-sm hover:bg-surface disabled:opacity-40 disabled:hover:bg-transparent disabled:cursor-not-allowed ${
              row.danger ? "text-status-failed" : "text-text-secondary"
            } ${cursor === i ? "bg-surface" : ""}`}
            onClick={() => {
              onClose();
              row.onSelect();
            }}
          >
            {row.label}
          </button>
        ),
      )}
    </div>
  );
}

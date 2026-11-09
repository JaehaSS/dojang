import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

/** hover 후 이만큼 머물러야 뜬다 — 지나가는 포인터마다 뜨면 화면이 깜박인다. */
export const TOOLTIP_OPEN_DELAY = 500;
/** 툴팁이 닫힌 직후 이 안에 다른 트리거로 옮기면 기다리지 않고 바로 띄운다(툴바 훑기). */
const WARM_WINDOW = 300;
const GAP = 6;
const EDGE = 4;

let lastClosedAt = 0;

interface TooltipOptions {
  /**
   * 툴팁 내용을 트리거의 설명(aria-describedby)으로 잇는다. 내용이 접근 이름과 같으면 끈다 —
   * 스크린 리더가 같은 말을 두 번 읽는다.
   */
  describe?: boolean;
}

/**
 * 공용 툴팁 (DESIGN.md `components.Tooltip`, 고도 2).
 *
 * 네이티브 `title`은 1초가량 늦게 OS 모양으로 뜨고, 키보드 포커스로는 뜨지 않는다.
 * 이 훅은 hover 500ms 또는 키보드 포커스에 바로 띄우고, 누르거나 Esc·스크롤에 닫는다.
 * 마우스로 눌러 생긴 포커스에는 띄우지 않는다 — 방금 누른 버튼 위에 이름이 뜨면 방해만 된다.
 *
 * 래퍼 요소 없이 트리거에 핸들러를 직접 붙이도록 훅으로 둔다. 감싸는 span은 flex 자식의
 * 크기·shrink 규칙을 바꾼다.
 */
export function useTooltip<T extends HTMLElement>(content: ReactNode, { describe = true }: TooltipOptions = {}) {
  const ref = useRef<T | null>(null);
  const tipRef = useRef<HTMLDivElement | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const openRef = useRef(false);
  const pointerDown = useRef(false);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const id = useId();

  const show = useCallback((immediate: boolean) => {
    window.clearTimeout(timer.current);
    const go = () => {
      openRef.current = true;
      setOpen(true);
    };
    if (immediate || Date.now() - lastClosedAt < WARM_WINDOW) go();
    else timer.current = window.setTimeout(go, TOOLTIP_OPEN_DELAY);
  }, []);

  const hide = useCallback(() => {
    window.clearTimeout(timer.current);
    if (openRef.current) lastClosedAt = Date.now();
    openRef.current = false;
    setOpen(false);
    setPos(null);
  }, []);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") hide();
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", hide, true);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", hide, true);
    };
  }, [open, hide]);

  // 아래에 놓고, 창 밑으로 넘치면 위로 뒤집는다. 가로는 트리거 중앙에 맞추되 창 안으로 민다.
  useLayoutEffect(() => {
    if (!open || !ref.current || !tipRef.current) return;
    const r = ref.current.getBoundingClientRect();
    const { width: w, height: h } = tipRef.current.getBoundingClientRect();
    let top = r.bottom + GAP;
    if (top + h > window.innerHeight - EDGE) top = Math.max(EDGE, r.top - GAP - h);
    const left = Math.min(Math.max(EDGE, r.left + r.width / 2 - w / 2), Math.max(EDGE, window.innerWidth - EDGE - w));
    setPos({ left, top });
  }, [open]);

  const triggerProps = {
    onMouseEnter: () => show(false),
    onMouseLeave: () => {
      pointerDown.current = false;
      hide();
    },
    onMouseDown: () => {
      pointerDown.current = true;
      hide();
    },
    onFocus: () => {
      if (!pointerDown.current) show(true);
    },
    onBlur: () => {
      pointerDown.current = false;
      hide();
    },
    "aria-describedby": open && describe ? id : undefined,
  };

  const node =
    open && content
      ? createPortal(
          <div
            ref={tipRef}
            id={id}
            role="tooltip"
            className="pointer-events-none fixed z-[60] max-w-xs rounded-md border border-border-strong bg-raised px-2 py-1 text-xs text-text shadow-e2"
            style={pos ? { left: pos.left, top: pos.top } : { left: 0, top: 0, visibility: "hidden" }}
          >
            {content}
          </div>,
          document.body,
        )
      : null;

  return { ref, triggerProps, node, hide };
}

import { useEffect, useRef, useState, type ReactNode } from "react";

interface DialogProps {
  /** 스크린 리더가 읽을 다이얼로그 이름. */
  label: string;
  onClose: () => void;
  /** 패널 크기·레이아웃 — 폭은 다이얼로그마다 다르므로 호출부가 정한다. */
  className?: string;
  children: ReactNode;
}

/**
 * 모달 다이얼로그 껍데기 (DESIGN.md `components.Dialog`, 고도 3).
 *
 * 세 가지를 한 곳에서 보장한다.
 * - 바깥을 누르면 닫힌다. `mousedown` 대상이 배경 자신일 때만 — 패널 안에서 드래그를 시작해
 *   바깥에서 놓아도 닫히지 않는다.
 * - Esc로 닫힌다. 호출부가 입력란에서 Esc를 따로 잡으면 두 번 닫히므로 잡지 말 것.
 * - 닫힌 뒤 연 버튼으로 포커스가 돌아간다.
 */
export function Dialog({ label, onClose, className = "w-96", children }: DialogProps) {
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  // 연 요소는 render 시점에 잡는다. effect에서 읽으면 자식의 autoFocus·초기 포커스가 이미
  // 포커스를 옮긴 뒤라 다이얼로그 안의 입력란을 "연 요소"로 기억한다.
  const [opener] = useState(() =>
    typeof document !== "undefined" && document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) closeRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      if (opener?.isConnected) opener.focus();
    };
  }, [opener]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={label}
        className={`max-w-full rounded-xl border border-border-strong bg-raised shadow-e3 ${className}`}
      >
        {children}
      </div>
    </div>
  );
}

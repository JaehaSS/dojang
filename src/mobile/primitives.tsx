import type { ReactNode } from "react";
import type { Tone } from "./status";
import { TaskStatusIcon } from "../components/ide/TaskStatusIcon";

// 모바일 공용 프리미티브. 데스크톱 4열 셸의 컴포넌트는 폭 전제가 달라 재사용하지 않는다.
// 터치 대상은 최소 44px — 손가락으로 눌러 오탭이 나면 승인 화면에서 사고가 된다.

const TONE_TEXT: Record<Tone, string> = {
  running: "text-status-running",
  awaiting: "text-status-awaiting",
  question: "text-status-question",
  done: "text-status-done",
  failed: "text-status-failed",
  muted: "text-text-muted",
};

const TONE_BG: Record<Tone, string> = {
  running: "bg-status-running",
  awaiting: "bg-status-awaiting",
  question: "bg-status-question",
  done: "bg-status-done",
  failed: "bg-status-failed",
  muted: "bg-text-muted",
};


export function StatusPill({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    <span className={`inline-flex shrink-0 items-center gap-1 text-xs ${TONE_TEXT[tone]}`}>
      {/* 모양이 다른 상태 아이콘 — 색 단독 인코딩 금지(DESIGN.md Do #2). 데스크톱과 같은 컴포넌트다. */}
      <TaskStatusIcon tone={tone} label="" size={12} decorative />
      {children}
    </span>
  );
}

/**
 * 카드 왼쪽 상태 스트립 (DESIGN.md Do #1 · compact 3px).
 * 배지만으론 시선 이동 비용이 크다 — 폰은 훑는 거리가 짧아 효과가 더 크다.
 */
export function StatusStrip({ tone }: { tone: Tone }) {
  return <span aria-hidden className={`absolute inset-y-0 left-0 w-[3px] ${TONE_BG[tone]}`} />;
}

export function MobileList({ children }: { children: ReactNode }) {
  return <ul className="divide-y divide-border">{children}</ul>;
}

export function Row({
  onClick,
  children,
  ariaLabel,
}: {
  onClick?: () => void;
  children: ReactNode;
  ariaLabel?: string;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        aria-label={ariaLabel}
        className="flex min-h-[56px] w-full flex-col gap-1 px-4 py-3 text-left active:bg-raised"
      >
        {children}
      </button>
    </li>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return <div className="px-6 py-12 text-center text-sm text-text-muted">{children}</div>;
}

export function Spinner({ label }: { label: string }) {
  return <div className="px-6 py-12 text-center text-sm text-text-muted">{label}</div>;
}

/** 하단에서 올라오는 시트. 승인처럼 되돌리기 어려운 동작의 확인 단계로 쓴다. */
export function BottomSheet({
  open,
  title,
  onClose,
  children,
}: {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
}) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex flex-col justify-end">
      <button
        type="button"
        aria-label="닫기"
        onClick={onClose}
        className="flex-1 bg-black/60"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        // elevation-3 (DESIGN.md): raised bg + borderStrong + inset 상단 하이라이트.
        // 다크에서 "들린" 느낌을 내는 유일한 수단이다.
        className="rounded-t-xl border-t border-border-strong bg-raised px-4 pt-4"
        style={{
          paddingBottom: "calc(env(safe-area-inset-bottom) + 1rem)",
          boxShadow: "inset 0 1px 0 rgba(255,255,255,0.06), 0 -8px 32px rgba(0,0,0,0.6)",
        }}
      >
        <div className="mb-3 text-lg font-semibold text-text">{title}</div>
        {children}
      </div>
    </div>
  );
}

export function Button({
  onClick,
  children,
  variant = "default",
  disabled,
}: {
  onClick?: () => void;
  children: ReactNode;
  variant?: "default" | "primary" | "danger";
  disabled?: boolean;
}) {
  const style =
    variant === "primary"
      ? "bg-primary text-black active:bg-primary-hover"
      : variant === "danger"
        ? "border border-dangerborder bg-dangerbg text-text"
        : "border border-border text-text active:bg-raised";
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      // radius는 DESIGN.md Shapes의 button=md(6). 높이는 데스크톱 36px 대신 터치 최소 44px.
      className={`min-h-[44px] w-full rounded-md px-4 text-md font-medium disabled:opacity-50 ${style}`}
    >
      {children}
    </button>
  );
}

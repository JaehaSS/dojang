import type { ReactElement, ReactNode } from "react";

/** 플로팅 채널 카드의 공통 외피 — 채널이 둘 이상이 되면서 스타일을 한 곳에 모았다. */
const SHELL =
  "side-panel-scrollbars-hidden pointer-events-auto flex flex-col overflow-y-auto rounded-xl border border-border-strong bg-raised shadow-e3";

/**
 * 세션 위에 떠 있는 채널 한 장.
 *
 * 카드는 콘텐츠 높이로 접힌다 — 배경을 가진 빈 카드가 세션을 막아 보이지 않게 한다(설계 0018 D9).
 * 스택 컨테이너가 `pointer-events-none`이라 카드만 클릭을 받는다.
 */
export function ChannelCard({
  label,
  className,
  children,
}: {
  label: string;
  className?: string;
  children: ReactNode;
}): ReactElement {
  return (
    <aside className={className ? `${SHELL} ${className}` : SHELL} aria-label={label}>
      {children}
    </aside>
  );
}

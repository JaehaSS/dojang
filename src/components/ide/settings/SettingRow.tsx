import { createContext, useContext, useEffect, useRef } from "react";
import { type SettingRisk } from "./settings-catalog";

/**
 * 검색이 지목한 항목 id. 섹션·행이 스스로 읽어 자기 차례에 스크롤·강조한다 —
 * 패널이 DOM을 뒤지지 않게 하려는 것이다.
 */
export const SettingHighlight = createContext<string | null>(null);

/** 설정 스위치 — 패널 어디서나 같은 모양이어야 한다(에디터 탭도 이것을 쓴다). */
export function Switch({
  on,
  onClick,
  label,
}: {
  on: boolean;
  onClick: () => void;
  /** 행 제목이 스크린리더에 닿지 않는 자리(중첩 컨트롤)에서만 쓴다. */
  label?: string;
}) {
  return (
    <button
      onClick={onClick}
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`w-11 h-6 shrink-0 rounded-full relative transition-colors ${on ? "bg-primary" : "bg-border"}`}
    >
      <span
        className={`absolute top-0.5 w-5 h-5 rounded-full bg-bg transition-all ${on ? "left-[22px]" : "left-0.5"}`}
      />
    </button>
  );
}

/** 위험 배지 — 색만으로 구분하지 않는다. 글자가 본체고 색은 보조다. */
export function RiskBadge({ risk }: { risk: SettingRisk }) {
  const spec =
    risk === "cost"
      ? { label: "비용", cls: "text-status-awaiting border-status-awaiting", title: "켜거나 올리면 벤더 호출·요금이 늘어납니다" }
      : { label: "안전장치", cls: "text-status-failed border-status-failed", title: "끄면 보호가 사라집니다" };
  return (
    <span
      title={spec.title}
      className={`shrink-0 rounded border px-1 text-[11px] leading-[15px] ${spec.cls}`}
    >
      {spec.label}
    </span>
  );
}

/** 강조 대상이면 스크롤해 올리고 링을 두른다. jsdom에는 scrollIntoView가 없다. */
export function useHighlight(id?: string) {
  const highlight = useContext(SettingHighlight);
  const hit = !!id && highlight === id;
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (hit) ref.current?.scrollIntoView?.({ block: "center", behavior: "smooth" });
  }, [hit]);
  return { ref, ring: hit ? "ring-2 ring-primary" : "" };
}

/**
 * 섹션 — 굵은 제목 + 설명, 그 아래 여러 행. 행이 섹션 밖에 직접 놓이는 경우를 만들지 않는다.
 * 제목 굵기가 섹션과 행에서 다른 것이 그룹 경계를 보이게 하는 유일한 장치다.
 */
export function SettingSection({
  id,
  title,
  hint,
  risk,
  children,
}: {
  /** 카탈로그의 항목 id. 검색이 지목할 수 있는 자리에만 준다. */
  id?: string;
  title: string;
  hint?: React.ReactNode;
  risk?: SettingRisk;
  children: React.ReactNode;
}) {
  const { ref, ring } = useHighlight(id);
  return (
    <section
      ref={ref}
      data-setting-id={id}
      className={`scroll-mt-4 rounded-md border-b border-border px-1 pb-4 pt-5 first:pt-0 ${ring}`}
    >
      <div className="mb-3">
        <div className="flex items-center gap-1.5">
          <div className="font-medium text-text">{title}</div>
          {risk && <RiskBadge risk={risk} />}
        </div>
        {hint && <div className="text-xs text-text-muted">{hint}</div>}
      </div>
      <div className="space-y-3">{children}</div>
    </section>
  );
}

/** 행 — 제목(평문) + 설명 + 우측 컨트롤. 항상 섹션 안에 있다. */
export function SettingRow({
  id,
  title,
  hint,
  risk,
  children,
}: {
  /** 카탈로그의 항목 id. 검색이 지목할 수 있는 자리에만 준다. */
  id?: string;
  title: string;
  hint?: React.ReactNode;
  risk?: SettingRisk;
  /** 우측 컨트롤. 스위치·입력·읽기 전용 값 어느 것이든. */
  children?: React.ReactNode;
}) {
  const { ref, ring } = useHighlight(id);
  return (
    <div
      ref={ref}
      data-setting-id={id}
      className={`flex items-start justify-between gap-4 rounded-md py-2 ${ring}`}
    >
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <div className="text-text">{title}</div>
          {risk && <RiskBadge risk={risk} />}
        </div>
        {hint && <div className="text-xs text-text-muted">{hint}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2 pt-0.5">{children}</div>}
    </div>
  );
}

/**
 * 탭 요약 — 그 탭의 현재 값을 한 줄로. 값은 탭 컴포넌트가 자기 상태에서 만든다.
 * 패널로 끌어올리면 탭을 나눈 의미가 없어진다.
 */
export function TabSummary({ items }: { items: (string | null)[] }) {
  const shown = items.filter((item): item is string => !!item);
  if (shown.length === 0) return null;
  return (
    <div className="mb-4 rounded-md border border-border bg-raised px-3 py-2 text-xs text-text-secondary">
      {shown.join(" · ")}
    </div>
  );
}

/** 설정 탭 공통 껍데기 — 스크롤 영역과 본문 폭을 한 곳에서 정한다. */
export function SettingsTabShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex-1 overflow-auto p-6">
      <div className="mx-auto max-w-3xl">{children}</div>
    </div>
  );
}

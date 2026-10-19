import { useEffect, useRef, type ReactElement, type ReactNode } from "react";
import { Icon } from "./icons";

export type CodeTab = "activity" | "file" | "preview" | "design" | "question";

interface Props {
  active: CodeTab;
  onActivate: (tab: CodeTab) => void;
  /** 프리뷰는 로컬 작업에서만 열린다 — 원격에서는 탭 자체를 만들지 않는다. */
  previewAvailable: boolean;
  /**
   * 에디터가 별도 창에 나가 있는지 — 그동안 파일 탭은 사라진다. 창이 하나뿐인 자리다.
   *
   * 빼내는 손잡이는 세션 헤더로 옮겼다. 여기 있으면 코드 열을 먼저 열어야 보였기 때문이다.
   */
  editorPoppedOut: boolean;
  /**
   * 코드 열 닫기 — 소환된 면은 **자기 위에** 닫기를 갖는다(Do #11). 헤더 칩 재클릭과
   * ⌥⌘S는 보조 경로다.
   */
  onClose?: () => void;
  /**
   * 작업정보 — 코드 열이 열려 있는 동안 플로팅 채널이 흡수되는 고정 탭(ADR 0066 결정 3).
   * 닫을 수 없고 항상 맨 앞이다. 거처가 둘이면 어느 쪽이 최신인지 사용자가 판단해야 한다.
   */
  activity: ReactNode;
  file: ReactNode;
  preview: ReactNode;
  design?: ReactNode;
  question?: ReactNode;
}

const LABELS: Record<CodeTab, string> = {
  activity: "작업정보",
  file: "파일",
  preview: "프리뷰",
  design: "Design",
  question: "따로 질문",
};

/**
 * 코드 열의 탭 — 2열 레이아웃 오른쪽에 사는 주 작업물들.
 *
 * 파일·프리뷰가 사이드 패널이 아니라 여기 있는 이유는, 둘이 곁눈으로 보는 보조 정보가
 * 아니라 **작업 대상**이기 때문이다. 가로 예산을 상시 점유할 자격은 주 작업물에만 있다(설계 0044).
 *
 * Diff는 여기 탭이 아니다. 변경 목록은 트리 열의 `변경` 렌즈이고, 본문은 파일 탭 안의
 * `diff:<path>` 에디터 탭이다(ADR 2026-10-08). 목록을 이 열에 두던 시절(2026-09-06~10-08)에는
 * 480px 열 하나가 목록만 받고 본문이 세션 열을 덮었다.
 *
 * 작업정보만 성격이 다르다. 이 열이 열려 있는 동안에는 세션 위에 뜰 자리가 없어 여기 얹히고,
 * 열이 닫히면 플로팅 채널로 되돌아간다 — 승격과 흡수가 대칭이다(ADR 0066 결정 4).
 *
 * 기존 탭의 자식은 언마운트하지 않고 감춘다 — Monaco와 프리뷰 웹뷰는 재마운트 비용이 크고,
 * 스크롤·선택 상태를 잃으면 탭 전환이 "돌아왔다"가 아니라 "다시 열었다"가 된다.
 * Design은 같은 네이티브 표면을 빌리므로 예외다. 호출부가 Preview를 내리고,
 * Design 서버는 활성 탭의 수명에 맞춰 시작·정리한다.
 */
export function CodeColumnTabs({
  active,
  onActivate,
  previewAvailable,
  editorPoppedOut,
  onClose,
  activity,
  file,
  preview,
  design,
  question,
}: Props): ReactElement {
  const tabListRef = useRef<HTMLDivElement>(null);
  // 작업정보는 도구가 아니라 상태다 — 다른 탭과 슬롯을 경쟁하지 않도록 맨 앞에 상주한다.
  const all: CodeTab[] = previewAvailable
    ? ["activity", "file", "preview"]
    : ["activity", "file"];
  if (question) all.push("question");
  if (previewAvailable && design) all.splice(all.indexOf("preview") + 1, 0, "design");
  // 에디터가 별도 창에 나가 있으면 파일 탭은 여기 없다 — 같은 코드를 두 자리에 두지 않는다.
  const tabs = editorPoppedOut ? all.filter((tab) => tab !== "file") : all;
  // 열 수 없는 탭이 활성으로 남으면 빈 화면이 된다 — 첫 탭으로 되돌린다.
  const current: CodeTab = tabs.includes(active) ? active : (tabs[0] ?? "activity");
  useEffect(() => {
    const tabList = tabListRef.current;
    if (!tabList) return;
    const revealSelected = () => {
      const selected = tabList.querySelector<HTMLElement>('[aria-selected="true"]');
      selected?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
    };
    revealSelected();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(revealSelected);
    observer.observe(tabList);
    return () => observer.disconnect();
  }, [current]);
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center border-b border-border">
        <div ref={tabListRef} role="tablist" className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto px-2 py-1">
          {tabs.map((tab) => (
            <button
              key={tab}
              role="tab"
              aria-selected={current === tab}
              tabIndex={current === tab ? 0 : -1}
              onKeyDown={(event) => {
                const index = tabs.indexOf(current);
                const next = event.key === "ArrowRight" ? tabs[(index + 1) % tabs.length]
                  : event.key === "ArrowLeft" ? tabs[(index + tabs.length - 1) % tabs.length]
                  : event.key === "Home" ? tabs[0] : event.key === "End" ? tabs[tabs.length - 1] : null;
                if (!next) return;
                event.preventDefault();
                onActivate(next);
                tabListRef.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[tabs.indexOf(next)]?.focus();
              }}
              className={`shrink-0 whitespace-nowrap border-b-2 px-2 py-0.5 text-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary ${
                current === tab
                  ? "border-primary text-primary-bright font-medium"
                  : "border-transparent text-text-secondary hover:text-text"
              }`}
              onClick={() => onActivate(tab)}
            >
              {LABELS[tab]}
            </button>
          ))}
        </div>
        {onClose && (
          <div className="shrink-0 border-l border-border px-1 py-1">
            <button
              className="rounded p-1 text-text-secondary hover:bg-raised hover:text-text"
              onClick={onClose}
              title="코드 열 닫기 (⌥⌘S)"
              aria-label="코드 열 닫기"
            >
              <Icon name="x" size={14} />
            </button>
          </div>
        )}
      </div>
      <div className={`${current === "activity" ? "flex" : "hidden"} min-h-0 flex-1 flex-col`}>
        {activity}
      </div>
      <div className={`${current === "file" ? "flex" : "hidden"} min-h-0 flex-1 flex-col`}>
        {file}
      </div>
      {previewAvailable && (
        <div className={`${current === "preview" ? "flex" : "hidden"} min-h-0 flex-1 flex-col`}>
          {preview}
        </div>
      )}
      {question && <div className={`${current === "question" ? "flex" : "hidden"} min-h-0 flex-1 flex-col`} inert={current !== "question"}>
        {question}
      </div>}
      {previewAvailable && design && <div className={`${current === "design" ? "flex" : "hidden"} min-h-0 flex-1 flex-col`} inert={current !== "design"}>{design}</div>}
    </div>
  );
}

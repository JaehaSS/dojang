import { useState, type ReactElement } from "react";
import {
  activityEntries,
  currentOperation,
  subagentEntries,
  type ActivityItem,
  type SubagentEntry,
} from "../../lib/activity";
import type { ConvoStatus, Task } from "../../lib/ipc";
import { Icon } from "./icons";
import { WorkContextPanel, type WorkContextDiff } from "./WorkContextPanel";

/** 플로팅 채널은 좁은 폭의 오버레이라 목록 섹션을 접는다(설계 0018 D3). */
export type ActivityDensity = "rail" | "panel";

interface Props {
  task: Task;
  runtime: "local" | "remote";
  diff: WorkContextDiff;
  items: ActivityItem[];
  busy: boolean;
  activity: ConvoStatus | null;
  onRefreshDiff: () => void;
  /** 플로팅 채널 전용 접기 — 코드 열 탭 밀도에서는 전달되지 않는다(Don't #12). */
  onCollapse?: () => void;
  onOpenConversation?: () => void;
  /** 서브 에이전트 행 클릭 시 전용 탭 열기. */
  onOpenSubagent: (toolId: string) => void;
  /** 채널 밀도에서 최근 활동 진입점 클릭 시 사이드 패널 작업정보 탭 열기. */
  onOpenRecentActivity?: () => void;
  density?: ActivityDensity;
}

const labelForState = (busy: boolean, state: ConvoStatus["state"] | undefined) => {
  if (!busy) return "최근 작업";
  if (state === "starting") return "에이전트 시작 중";
  if (state === "ended_without_result") return "로컬 실행 종료 — 응답을 받지 못했습니다";
  if (state === "unknown") return "실행 상태 확인 불가";
  return "에이전트 작업 중";
};

const colorForState = (busy: boolean, state: ConvoStatus["state"] | undefined) => {
  if (state === "ended_without_result") return "text-status-awaiting";
  if (state === "unknown") return "text-text-muted";
  if (busy) return "text-primary-bright";
  return "text-status-done";
};

/** 서브 에이전트 1건의 상태 표기 — 턴이 끝났는데 running이면 "중단"으로 표면화
 *  (프린트 모드 벤더 CLI는 턴 종료 시 백그라운드 서브 에이전트를 남기지 않는다). */
const subagentStatus = (busy: boolean, s: SubagentEntry) => {
  if (s.state === "failed") return { mark: "×", color: "text-status-failed", note: "실패" };
  if (s.state === "done") return { mark: "✓", color: "text-status-done", note: "완료" };
  if (!busy) return { mark: "○", color: "text-status-awaiting", note: "미완료 — 턴 종료로 중단됨" };
  return { mark: "→", color: "text-primary-bright", note: "작업 중" };
};

/** 플로팅 채널에서 하위 에이전트를 한 줄로 압축한다 — "완료 3 · 진행 1" 형태. */
const subagentSummary = (busy: boolean, list: SubagentEntry[]): string => {
  const done = list.filter((s) => s.state === "done").length;
  const failed = list.filter((s) => s.state === "failed").length;
  const open = list.length - done - failed;
  const parts: string[] = [];
  if (done > 0) parts.push(`완료 ${done}`);
  if (open > 0) parts.push(busy ? `진행 ${open}` : `미완료 ${open}`);
  if (failed > 0) parts.push(`실패 ${failed}`);
  return parts.join(" · ");
};

/** 플로팅 채널 전용 접이식 섹션 헤더 — 패널 밀도에서는 쓰지 않는다. */
function SectionToggle(props: {
  id: string;
  label: string;
  summary: string;
  open: boolean;
  onToggle: () => void;
}): ReactElement {
  return (
    <button
      type="button"
      className="mb-2 flex w-full items-center gap-2 text-xs font-medium text-text-secondary hover:text-text"
      onClick={props.onToggle}
      aria-expanded={props.open}
      aria-controls={props.id}
    >
      <span>{props.label}</span>
      {props.summary && <span className="font-normal text-text-muted">{props.summary}</span>}
      <span className={`ml-auto transition-transform ${props.open ? "" : "-rotate-90"}`}>
        <Icon name="chevronDown" size={14} />
      </span>
    </button>
  );
}

/** 에디터를 보고 있어도 선택 작업의 환경과 최근 도구 이력을 보여 주는 보조 패널.
 *  플로팅 채널이 유일한 셸이다(ActivityRail) — 사이드 패널은 설계 0044에서 폐지했다. */
export function ActivityPanel({
  task,
  runtime,
  diff,
  items,
  busy,
  activity,
  onRefreshDiff,
  onOpenConversation,
  onOpenSubagent,
  onOpenRecentActivity,
  onCollapse,
  density = "panel",
}: Props) {
  const entries = activityEntries(items).slice(0, 30);
  const subagents = subagentEntries(items).slice(0, 10);
  const operation = currentOperation(items, activity?.last_operation);
  const state = activity?.state;
  const rail = density === "rail";
  // 플로팅 채널에서만 접는다 — 패널 밀도는 항상 펼친 상태로 렌더한다.
  const [subagentsOpen, setSubagentsOpen] = useState(false);
  const showSubagents = !rail || subagentsOpen;

  return (
    // 채널은 콘텐츠 높이로 접힌다 — flex-1로 늘리면 빈 배경이 세션을 가린다. 패널은 기존대로 높이를 채운다.
    <div className={rail ? "flex flex-col" : "flex-1 flex flex-col min-h-0"}>
      <WorkContextPanel
        task={task}
        runtime={runtime}
        diff={diff}
        onRefresh={onRefreshDiff}
        onCollapse={onCollapse}
      />

      <section className="border-b border-border px-3 py-3 text-xs" aria-labelledby="activity-heading">
        <h2 id="activity-heading" className="mb-2 font-medium text-text-secondary">
          현재 활동
        </h2>
        <div className="flex items-center gap-2 text-text-secondary">
          <span className={colorForState(busy, state)}>●</span>
          <span>{labelForState(busy, state)}</span>
        </div>
        {operation ? (
          <div className="font-code text-text-muted break-words">최근: {operation}</div>
        ) : busy ? (
          <div className="text-text-muted">도구 호출을 기다리는 중…</div>
        ) : null}
      </section>

      {subagents.length > 0 && (
        <section className="border-b border-border px-3 py-3 text-xs space-y-1.5" aria-labelledby="subagents-heading">
          {rail ? (
            <SectionToggle
              id="subagents-list"
              label="하위 에이전트"
              summary={subagentSummary(busy, subagents)}
              open={subagentsOpen}
              onToggle={() => setSubagentsOpen((v) => !v)}
            />
          ) : (
            <h2 id="subagents-heading" className="mb-2 font-medium text-text-secondary">
              하위 에이전트
            </h2>
          )}
          {showSubagents && subagents.map((s) => {
            const st = subagentStatus(busy, s);
            return (
              <button
                key={s.id}
                className="block w-full text-left space-y-0.5 rounded hover:bg-raised px-1 py-0.5 -mx-1"
                onClick={() => onOpenSubagent(s.id)}
                title="서브 에이전트 탭 열기"
              >
                <div className="flex items-center gap-1.5">
                  <span className={st.color}>{st.mark}</span>
                  <span className="text-text-secondary truncate">{s.title}</span>
                  <span className="ml-auto shrink-0 text-text-muted">{st.note}</span>
                </div>
                {s.lastOp && (
                  <div className="pl-4 font-code text-text-muted break-words">{s.lastOp}</div>
                )}
              </button>
            );
          })}
        </section>
      )}

      {/* 최근 활동은 길어서 채널에 두면 세션을 가린다 — 채널에는 진입점만 두고 본문은 사이드 패널이 맡는다. */}
      {rail ? (
        onOpenRecentActivity && (
          <button
            type="button"
            className="flex shrink-0 items-center gap-2 border-t border-border px-3 py-2.5 text-xs text-text-secondary hover:bg-surface hover:text-text"
            onClick={onOpenRecentActivity}
            title="사이드 패널에서 최근 활동 보기"
          >
            <span>최근 활동</span>
            {entries.length > 0 && <span className="text-text-muted">{entries.length}건</span>}
            <span className="ml-auto">
              <Icon name="chevronRight" size={14} />
            </span>
          </button>
        )
      ) : (
        <section
          className="flex-1 min-h-0 overflow-auto px-3 py-3 space-y-2"
          aria-labelledby="recent-activity-heading"
        >
          <h2 id="recent-activity-heading" className="mb-2 text-xs font-medium text-text-secondary">
            최근 활동
          </h2>
          {entries.length === 0 ? (
            <div className="text-xs text-text-muted leading-relaxed">
              {busy ? "활동 이벤트를 기다리는 중입니다." : "이 대화에서 기록된 도구 활동이 없습니다."}
            </div>
          ) : (
            entries.map((entry) => (
              <div key={entry.index} className="border-l border-border pl-2.5 py-0.5 text-xs space-y-0.5">
                <div className="flex items-center gap-1.5">
                  <span
                    className={
                      entry.state === "failed"
                        ? "text-status-failed"
                        : entry.state === "done"
                          ? "text-status-done"
                          : "text-primary-bright"
                    }
                  >
                    {entry.state === "failed" ? "×" : entry.state === "done" ? "✓" : "→"}
                  </span>
                  <span className="text-text-secondary truncate">{entry.title}</span>
                </div>
                {entry.detail && <div className="font-code text-text-muted break-words">{entry.detail}</div>}
              </div>
            ))
          )}
        </section>
      )}

      {/* 플로팅 채널 아래에 세션이 있으므로 별도 진입점은 패널 밀도에서만 노출. */}
      {!rail && onOpenConversation && (
        <button
          className="h-9 shrink-0 border-t border-border text-xs text-text-secondary hover:text-text"
          onClick={onOpenConversation}
        >
          대화 전체 보기
        </button>
      )}
    </div>
  );
}

import { useEffect, useId, useState, type ReactElement, type ReactNode } from "react";
import { ago, stamp } from "../../lib/fmt";
import { taskToolCost, type Task, type ToolCostReport } from "../../lib/ipc";
import { lastRunLabel, taskStatusLabel, taskTone } from "../../lib/task-status";
import { Badge } from "../ui";
import { Icon, type IconName } from "./icons";
import { AmbiguityBadge } from "./InterviewPanel";
import { IconButton } from "../ui";

export type WorkContextDiff =
  | { state: "loading" }
  | { state: "ready"; value: string }
  | { state: "error" };

export interface DiffStatSummary {
  files: number;
  additions: number;
  deletions: number;
}

interface Props {
  task: Task;
  runtime: "local" | "remote";
  diff: WorkContextDiff;
  onRefresh: () => void;
  /** 플로팅 채널에서만 온다 — 환경 헤더의 ✕(작업정보 접기). 코드 열 탭에는 닫기가 없다(Don't #12). */
  onCollapse?: () => void;
}

interface ContextRowProps {
  icon: IconName;
  label: string;
  children: ReactNode;
  title?: string;
}

const countFrom = (value: string, pattern: RegExp): number => {
  const match = value.match(pattern);
  return match ? Number(match[1].replace(/,/g, "")) : 0;
};

export function parseDiffStat(value: string): DiffStatSummary {
  return {
    files: countFrom(value, /([\d,]+) files? changed/),
    additions: countFrom(value, /([\d,]+) insertions?\(\+\)/),
    deletions: countFrom(value, /([\d,]+) deletions?\(-\)/),
  };
}

/** 상위 몇 개까지 보여줄지 — 꼬리는 개별 대응 가치가 없다. */
const TOOL_COST_LIMIT = 5;

const compact = (n: number): string => n.toLocaleString();

/**
 * 툴 결과가 컨텍스트에 더한 총량 — 귀속분 + 미귀속분.
 * 화면에 보이는 상위 몇 개가 아니라 **전체 행**을 근거로 센다. 잘라 보여 준 목록의 합을
 * "총량"이라 부르면 상위 5개 밖의 툴이 조용히 사라진다.
 */
export function totalToolTokens(report: ToolCostReport): number {
  return report.rows.reduce((sum, r) => sum + r.attributed_tokens, 0) + report.unattributed_tokens;
}

interface DisclosureProps {
  /** 펼쳤을 때 드러나는 본문 요소의 id — `aria-controls` 연결용. */
  bodyId: string;
  label: string;
  /** 접힌 상태에서도 남는 한 줄 요약. 오른쪽에 붙이려면 `ml-auto`를 직접 준다. */
  summary?: ReactNode;
  open: boolean;
  onToggle: () => void;
  title: string;
}

/**
 * 접기 헤더. 플로팅 채널은 폭 300px 오버레이라 세부까지 상시 펼쳐 두면 세션을 가린다 —
 * 기본은 요약 한 줄이고, 헤더를 눌러 본문을 연다(설계 0018 D3의 채널 압축 원칙).
 */
function Disclosure({ bodyId, label, summary, open, onToggle, title }: DisclosureProps): ReactElement {
  return (
    <button
      type="button"
      className="flex w-full items-center gap-2 text-left text-text-muted hover:text-text"
      onClick={onToggle}
      aria-expanded={open}
      aria-controls={bodyId}
      title={title}
    >
      <span className="shrink-0">{label}</span>
      {summary}
      <span className={`ml-auto shrink-0 transition-transform ${open ? "" : "-rotate-90"}`}>
        <Icon name="chevronDown" size={14} />
      </span>
    </button>
  );
}

/**
 * 툴별 컨텍스트 비용. **귀속하지 못한 값을 추정으로 덮지 않는 것**이 이 섹션의 요구사항이다
 * (계획 0033 BR-1) — `attributed_tokens`가 0이면 0이 아니라 대시를 그리고, 어느 툴에도
 * 귀속 못 한 증가분은 별도 행으로 분리한다.
 *
 * 기본은 총량 한 줄이다. "얼마나 먹었나"는 늘 궁금하지만 "어느 툴이 먹었나"는 줄일 대상을
 * 찾을 때만 필요하다 — 그때만 펼친다.
 */
export function ToolCostSection({ report }: { report: ToolCostReport | null }): ReactElement | null {
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  if (!report || report.rows.length === 0) return null;
  const rows = report.rows.slice(0, TOOL_COST_LIMIT);
  const total = totalToolTokens(report);
  return (
    <div className="mt-2 rounded-md border border-border bg-bg px-2.5 py-2 text-xs">
      <Disclosure
        bodyId={bodyId}
        label="툴 컨텍스트 비용"
        open={open}
        onToggle={() => setOpen((v) => !v)}
        title={open ? "툴별 내역 접기" : "어느 툴이 썼는지 펼치기"}
        summary={
          <span
            className="ml-auto truncate text-text-secondary"
            title="툴 결과로 늘어난 컨텍스트 합계 — 귀속분과 미귀속분을 더한 값이다"
          >
            {total > 0 ? `${compact(total)}토큰` : "—"}
          </span>
        }
      />
      {open && (
        <div id={bodyId} className="mt-1.5 border-t border-border pt-1.5">
          {report.peak_context_tokens > 0 && (
            <p className="mb-1 text-text-muted" title="이 작업에서 관측된 컨텍스트 점유 최댓값">
              최대 {compact(report.peak_context_tokens)}
            </p>
          )}
          <dl>
            {rows.map((row) => (
              <div key={row.tool} className="flex items-baseline gap-2 py-0.5">
                <dt className="min-w-0 flex-1 truncate font-code text-text-secondary">{row.tool}</dt>
                <dd className="shrink-0 text-text-muted">
                  {row.calls}회 · {compact(row.chars)}자
                  {row.calls_unknown_size > 0 && (
                    <span title="원문 크기를 알 수 없는 호출 — 이 문자 수 합계에서 빠져 있다">
                      {" "}
                      ({row.calls_unknown_size}건 미상)
                    </span>
                  )}{" "}
                  ·{" "}
                  {row.attributed_tokens > 0 ? (
                    <span className="text-text-secondary">{compact(row.attributed_tokens)}토큰</span>
                  ) : (
                    <span title="이 툴의 결과가 단독으로 관측된 구간이 없어 실측 토큰을 귀속할 수 없었다">
                      —
                    </span>
                  )}
                </dd>
              </div>
            ))}
          </dl>
          {report.unattributed_tokens > 0 && (
            <p
              className="mt-1 border-t border-border pt-1 text-text-muted"
              title="한 관측 구간에 툴 결과가 여럿이라 특정 툴에 귀속할 수 없는 증가분"
            >
              미귀속 {compact(report.unattributed_tokens)}토큰
            </p>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * 읽기 전용 파생 데이터라 패널 지역 상태로 충분하다 — diff처럼 상위까지 끌어올리지 않는다.
 * 작업이 진행 중이면 값이 계속 늘어나므로 새로고침으로 다시 읽을 수 있어야 한다.
 */
function useToolCost(taskId: number): { report: ToolCostReport | null; reload: () => void } {
  const [report, setReport] = useState<ToolCostReport | null>(null);
  const [nonce, setNonce] = useState(0);
  useEffect(() => {
    let alive = true;
    taskToolCost(taskId)
      .then((r) => {
        if (alive) setReport(r);
      })
      // 부가 정보라 실패해도 패널 본체를 막지 않는다. 직전 값은 지우지 않는다 —
      // 일시적 실패로 이미 보던 수치가 사라지면 되레 혼란스럽다.
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [taskId, nonce]);
  // 작업이 바뀌면 이전 작업 수치를 잠시라도 보여주지 않는다.
  useEffect(() => setReport(null), [taskId]);
  return { report, reload: () => setNonce((n) => n + 1) };
}

function ContextRow({ icon, label, children, title }: ContextRowProps): ReactElement {
  return (
    <div className="flex min-w-0 items-center gap-2.5 py-1.5" title={title}>
      <dt className="sr-only">{label}</dt>
      <Icon name={icon} size={16} />
      <dd className="min-w-0 flex-1 truncate text-text-secondary">{children}</dd>
    </div>
  );
}

function DiffSummary({ diff }: { diff: WorkContextDiff }): ReactElement {
  if (diff.state === "loading") {
    return <span className="text-text-muted">변경 사항 확인 중…</span>;
  }
  if (diff.state === "error") {
    return <span className="text-status-awaiting">변경 사항을 확인할 수 없음</span>;
  }
  const summary = parseDiffStat(diff.value);
  if (summary.files === 0) {
    return <span className="text-text-muted">변경 없음</span>;
  }
  return (
    <span className="flex items-center gap-2">
      <span>변경 사항</span>
      <span className="text-status-done">+{summary.additions}</span>
      <span className="text-status-failed">-{summary.deletions}</span>
      <span className="ml-auto text-text-muted">{summary.files}개 파일</span>
    </span>
  );
}


/** 절대 시각과 경과를 함께 — "12m 전"만으로는 어제인지 지난주인지 알 수 없고, 시각만으로는 감이 없다. */
function TimeMark({ label, at }: { label: string; at: number }): ReactElement {
  return (
    <div className="flex gap-1.5">
      <span className="shrink-0">{label}</span>
      <span className="truncate font-code">{stamp(at)}</span>
      <span className="ml-auto shrink-0">{ago(at)} 전</span>
    </div>
  );
}

/** 상태 점은 "지금 무엇"만 말한다. 세션이 언제 시작해 언제 마지막으로 멈췄는지는 별도 시간축이 필요하다. */
function SessionTimeline({ task }: { task: Task }): ReactElement {
  const lastRun = lastRunLabel(task);
  return (
    <div className="space-y-0.5 pb-1.5 pl-[26px] text-text-muted">
      <TimeMark label="시작" at={task.created_at} />
      {lastRun ? (
        <TimeMark label={lastRun} at={task.updated_at} />
      ) : (
        <div>아직 실행 기록 없음</div>
      )}
    </div>
  );
}

/**
 * 작업을 시작시킨 지시문. 세 줄을 넘으면 잘리는데, 그동안 전문을 보는 길은 `title` 툴팁뿐이었다 —
 * 마우스를 올려야만 보이는 것은 읽을 수 있는 것이 아니다. 펼치면 원문 줄바꿈까지 살려 보여 준다.
 *
 * 펼친 높이는 제한한다. 지시문이 길다고 채널을 독차지하면 아래 툴 비용까지 스크롤로 밀려난다.
 */
export function TaskGoalSection({ task }: { task: Task }): ReactElement {
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  const goal = task.instruction;
  const badge = task.ambiguity ? <AmbiguityBadge ambiguity={task.ambiguity} /> : undefined;
  return (
    <div className="mt-2 rounded-md border border-border bg-bg px-2.5 py-2 text-xs">
      {/* 목표가 비면 펼칠 것이 없다 — 아무 일도 하지 않는 토글을 두지 않는다. */}
      {goal ? (
        <Disclosure
          bodyId={bodyId}
          label="현재 목표"
          summary={badge}
          open={open}
          onToggle={() => setOpen((v) => !v)}
          title={open ? "목표 접기" : "목표 전문 펼치기"}
        />
      ) : (
        <div className="flex items-center gap-2 text-text-muted">
          <span>현재 목표</span>
          {badge}
        </div>
      )}
      <p
        id={bodyId}
        className={`mt-1 leading-relaxed text-text-secondary ${
          open ? "max-h-56 overflow-y-auto whitespace-pre-wrap" : "line-clamp-3"
        }`}
        title={goal || undefined}
      >
        {goal || "등록된 작업 목표가 없습니다."}
      </p>
    </div>
  );
}

const pathLeaf = (value: string): string => {
  const parts = value.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? value;
};

export function WorkContextPanel({ task, runtime, diff, onRefresh, onCollapse }: Props): ReactElement {
  const { report: toolCost, reload: reloadToolCost } = useToolCost(task.id);
  return (
    <section className="border-b border-border px-3 pb-3" aria-label="작업 환경">
      <div className="flex h-10 items-center text-xs">
        <h2 className="font-medium text-text-secondary">환경</h2>
        <IconButton
          size="sm"
          className="ml-auto"
          label="변경 사항 새로고침"
          title="변경 사항과 툴 비용 새로고침"
          onClick={() => {
            onRefresh();
            reloadToolCost();
          }}
        >
          <Icon name="refresh" size={14} />
        </IconButton>
        {onCollapse && (
          <IconButton size="sm" label="작업정보 접기" title="작업정보 접기 — 우상단 핸들로 다시 연다" onClick={onCollapse}>
            <Icon name="x" size={14} />
          </IconButton>
        )}
      </div>

      <dl className="text-xs">
        <ContextRow icon="diff" label="변경 사항">
          <DiffSummary diff={diff} />
        </ContextRow>
        <ContextRow icon="desktop" label="실행 환경">
          {runtime === "local" ? "로컬" : "원격"}
        </ContextRow>
        <ContextRow icon="branch" label="작업 브랜치" title={task.branch}>
          <span className="font-code">{task.branch}</span>
        </ContextRow>
        <ContextRow icon="branch" label="비교 기준" title={`비교 기준 ${task.base}`}>
          <span className="text-text-muted">기준</span>{" "}
          <span className="font-code">{task.base}</span>
        </ContextRow>
        <ContextRow icon="folder" label="작업 디렉터리" title={task.worktree_path}>
          <span className="font-code">{pathLeaf(task.worktree_path)}</span>
        </ContextRow>
        <ContextRow icon="clock" label="작업 상태">
          <Badge tone={taskTone(task)}>{taskStatusLabel(task)}</Badge>
        </ContextRow>
        <dt className="sr-only">세션 기록</dt>
        <dd>
          <SessionTimeline task={task} />
        </dd>
      </dl>

      <TaskGoalSection task={task} />

      <ToolCostSection report={toolCost} />
    </section>
  );
}

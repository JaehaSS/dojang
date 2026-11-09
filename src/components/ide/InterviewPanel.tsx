import { MAX_GRILL_ROUNDS, isStale as grillIsStale, type GrillState } from "../../lib/grill";
import { badgeTier, isStale as interviewIsStale, type InterviewState } from "../../lib/interview";
import type { AmbiguityScore, GrillQuestion, InterviewQuestion } from "../../lib/ipc";
import { Icon } from "./icons";
import { IconButton } from "../ui";

interface Props {
  /** 1단계 — 발산(깊게 파기) 상태. 리듀서는 채점과 별개다(Plan 0039 DR-P4). */
  grill: GrillState;
  /** 2단계 — 수렴(채점) 상태. */
  interview: InterviewState;
  /** 현재 컴포저 지시문 — 신선도 경고 판정용. */
  currentInstruction: string;
  /** 현재 컴포저 레포 — 지시문이 같아도 레포가 바뀌면 결과는 무효. */
  currentRepo: string;
  /** repo/지시문이 비어 시작 불가할 때 true. */
  disabled: boolean;
  // ── 1단계: 깊게 파기 ──
  onGrillStart: () => void;
  onGrillDraft: (value: string) => void;
  onGrillAnswer: () => void;
  onGrillAcceptRecommendation: () => void;
  onGrillDontKnow: () => void;
  onGrillEndNow: () => void;
  /** 개선된 지시문을 컴포저에 넣고 바로 2단계(채점)로 이어간다 — 통합 패널의 주 경로. */
  onGrillApplyAndScore: () => void;
  /** 개선된 지시문만 넣는다 — 채점 전에 손으로 더 고치고 싶을 때. */
  onGrillApplyInstruction: () => void;
  /** 적용 후에만 주어진다 — 되돌릴 원문이 없으면 undefined. */
  onGrillUndoInstruction?: () => void;
  onGrillSave: () => void;
  onGrillRetry: () => void;
  // ── 2단계: 채점 ──
  /** 현재 지시문을 바로 채점한다 — idle에서는 1단계를 건너뛰는 지름길이다. */
  onInterviewStart: () => void;
  onInterviewAnswer: (questionId: string, answer: string) => void;
  onInterviewCrystallize: () => void;
  onInterviewRetry: () => void;
  /** 패널을 닫아 두 단계를 모두 idle로 되돌린다 — 주어지지 않으면 ✕를 그리지 않는다(설계 0062 D-1). */
  onClose?: () => void;
}

/** 패널이 지금 어느 단계를 보여주는가 — 채점이 시작되면 파기 결과는 접힌 참고로 내려간다. */
export type InterviewStage = "idle" | "dig" | "score";

export function interviewStage(grill: GrillState, interview: InterviewState): InterviewStage {
  if (interview.phase !== "idle") return "score";
  if (grill.phase !== "idle") return "dig";
  return "idle";
}

const DIMENSION_LABELS: Record<string, string> = {
  goal: "목표",
  constraints: "제약",
  success: "완료 기준",
};

const TIER_CLASSES = {
  green: "bg-status-done/15 text-status-done",
  yellow: "bg-status-awaiting/15 text-status-awaiting",
  red: "bg-status-failed/15 text-status-failed",
} as const;

const PRIMARY_BUTTON =
  "rounded border border-border px-2 py-0.5 text-xs text-primary-bright hover:border-primary";
const SECONDARY_BUTTON =
  "rounded border border-border px-2 py-0.5 text-xs text-text-secondary hover:text-text";
const MUTED_BUTTON = "rounded border border-border px-2 py-0.5 text-xs text-text-muted hover:text-text";

/** 모호성 점수 배지 — 소프트 게이트(어떤 점수도 작업 생성을 막지 않는다). */
export function AmbiguityBadge({ ambiguity }: { ambiguity: AmbiguityScore }) {
  const tier = badgeTier(ambiguity.score);
  const unclear = (["goal", "constraints", "success"] as const)
    .filter((key) => ambiguity[key] < 0.8)
    .map((key) => DIMENSION_LABELS[key]);
  return (
    <span
      className={`rounded px-1.5 py-0.5 text-xs ${TIER_CLASSES[tier]}`}
      title={`목표 ${ambiguity.goal.toFixed(2)} · 제약 ${ambiguity.constraints.toFixed(2)} · 완료 기준 ${ambiguity.success.toFixed(2)}`}
    >
      모호성 {ambiguity.score.toFixed(2)}
      {unclear.length > 0 && ` · ${unclear.join("·")} 불명확`}
    </span>
  );
}

/** 단계 표시 — 어느 단계에 있는지, 앞 단계를 건너뛰었는지 한눈에 보인다. */
function StageChips({ stage }: { stage: InterviewStage }) {
  const chip = (active: boolean, label: string) => (
    <span
      className={`rounded px-1.5 py-0.5 ${
        active ? "bg-primary/15 text-primary-bright" : "text-text-muted"
      }`}
    >
      {label}
    </span>
  );
  return (
    <span className="flex items-center gap-1 text-xs">
      {chip(stage === "dig", "1 깊게 파기")}
      <span className="text-text-muted">→</span>
      {chip(stage === "score", "2 채점")}
    </span>
  );
}

// ---- 1단계: 깊게 파기 ----

/** 질문 카드 — 보기 목록 대신 모델의 추천 답 하나를 보여준다(설계 0026 DR-2).
 *  입력은 멀티라인 textarea다 — 한 줄 input은 짧은 답을 강제해 사고를 좁힌다. */
function GrillQuestionCard({
  question,
  draft,
  onDraft,
  onAnswer,
  onAcceptRecommendation,
  onDontKnow,
}: {
  question: GrillQuestion;
  draft: string;
  onDraft: (value: string) => void;
  onAnswer: () => void;
  onAcceptRecommendation: () => void;
  onDontKnow: () => void;
}) {
  return (
    <div className="rounded border border-border bg-bg p-2">
      <p className="text-xs text-text">{question.text}</p>
      {question.why && <p className="mt-1 text-xs text-text-muted">ⓘ {question.why}</p>}
      <div className="mt-1.5 rounded border border-border bg-raised/40 px-2 py-1.5">
        <p className="text-xs text-text-secondary">
          <span className="text-primary-bright">▸ 내 추천:</span> {question.recommendation}
        </p>
        <div className="mt-1.5 flex gap-1">
          <button type="button" className={`${PRIMARY_BUTTON} px-1.5`} onClick={onAcceptRecommendation}>
            추천대로
          </button>
          <button type="button" className={`${SECONDARY_BUTTON} px-1.5`} onClick={onDontKnow}>
            모르겠다
          </button>
        </div>
      </div>
      <textarea
        className="mt-1.5 w-full resize-y rounded border border-border bg-raised/40 px-2 py-1 text-xs text-text outline-none placeholder:text-text-muted focus:border-primary"
        rows={3}
        placeholder="답변 (여러 줄 가능) — 추천을 반박하거나 다르게 정의해도 됩니다"
        value={draft}
        onChange={(event) => onDraft(event.target.value)}
      />
      <div className="mt-1 flex justify-end">
        <button type="button" className={PRIMARY_BUTTON} onClick={onAnswer}>
          답변 →
        </button>
      </div>
    </div>
  );
}

/** 미해결 논점 — 종료 판정을 사용자가 검증할 수 있게 항상 노출한다(설계 0026 DR-3). */
function OpenThreads({ threads }: { threads: string[] }) {
  return (
    <details className="mt-1.5" open={threads.length > 0 && threads.length <= 5}>
      <summary className="cursor-pointer text-xs text-text-muted">
        남은 논점 {threads.length}개
      </summary>
      <ul className="mt-1 flex flex-col gap-0.5">
        {threads.map((thread) => (
          <li key={thread} className="text-xs text-text-secondary">
            · {thread}
          </li>
        ))}
      </ul>
    </details>
  );
}

/** 노트 본문과 저장 — 파기 단계의 done, 채점 단계의 접힌 참고 양쪽에서 같은 모양으로 쓴다. */
function GrillNoteBody({
  grill,
  onSave,
}: {
  grill: GrillState;
  onSave: () => void;
}) {
  if (!grill.note) return null;
  return (
    <>
      <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded border border-border bg-bg p-2 text-xs text-text-secondary">
        {grill.note.markdown}
      </pre>
      <div className="flex items-center gap-1.5">
        <button type="button" className={SECONDARY_BUTTON} onClick={onSave}>
          docs/explorations/에 저장
        </button>
        {grill.savedPath && <span className="text-xs text-text-muted">저장됨: {grill.savedPath}</span>}
      </div>
      {/* 저장 실패는 노트를 지우지 않고 여기서만 알린다 — 다시 시도하거나 복사해 갈 수 있다. */}
      {grill.error && grill.phase === "done" && (
        <p className="text-xs text-status-failed">저장 실패: {grill.error}</p>
      )}
      {grill.note.dropped > 0 && (
        <p className="text-xs text-text-muted">({grill.note.dropped}개 항목은 상한을 넘어 잘렸습니다)</p>
      )}
    </>
  );
}

// ---- 2단계: 채점 ----

function InterviewQuestionCard({
  question,
  answer,
  onAnswer,
}: {
  question: InterviewQuestion;
  answer: string;
  onAnswer: (questionId: string, answer: string) => void;
}) {
  return (
    <div className="rounded border border-border bg-bg p-2">
      <div className="mb-1 flex items-center gap-2 text-xs">
        <span className="rounded bg-primary/15 px-1.5 py-0.5 text-primary-bright">
          {DIMENSION_LABELS[question.dimension] ?? question.dimension}
        </span>
        <span className="text-text">{question.text}</span>
        {answer ? (
          <button
            type="button"
            className="ml-auto shrink-0 text-text-muted hover:text-text"
            onClick={() => onAnswer(question.id, "")}
          >
            건너뛰기
          </button>
        ) : (
          <span className="ml-auto shrink-0 text-text-muted">답 없으면 건너뜀</span>
        )}
      </div>
      {question.reason && <p className="mb-1.5 text-xs text-text-muted">{question.reason}</p>}
      {question.options.length > 0 && (
        <div className="mb-1.5 flex flex-wrap gap-1">
          {question.options.map((option) => (
            <button
              key={option}
              type="button"
              className={`rounded border px-1.5 py-0.5 text-xs ${
                answer === option
                  ? "border-primary text-primary-bright"
                  : "border-border text-text-secondary hover:text-text"
              }`}
              onClick={() => onAnswer(question.id, option)}
            >
              {option}
            </button>
          ))}
        </div>
      )}
      <input
        className="w-full rounded border border-border bg-raised/40 px-2 py-1 text-xs text-text outline-none placeholder:text-text-muted focus:border-primary"
        placeholder="직접 입력 (비우면 이 질문은 건너뜁니다)"
        value={answer}
        onChange={(event) => onAnswer(question.id, event.target.value)}
      />
    </div>
  );
}

/** 헤더 오른쪽 버튼의 문구 — 진행 중이면 상태, 아니면 다음에 할 수 있는 일. */
function headerLabel(stage: InterviewStage, grill: GrillState, interview: InterviewState): string {
  if (stage === "score") {
    if (interview.phase === "assessing") return "지시문 분석 중…";
    if (interview.phase === "crystallizing") return "점수 확정 중…";
    return "다시 채점";
  }
  if (stage === "dig") {
    if (grill.phase === "asking") return "질문 준비 중…";
    if (grill.phase === "noting") return "노트 정리 중…";
    return "다시 파기";
  }
  return "깊게 파기";
}

/** 컴포저 인터뷰 패널 — 한 진입점에서 두 단계를 순서대로 밟는다.
 *  1단계(깊게 파기)는 라운드당 질문 하나로 생각을 넓혀 노트와 개선된 지시문을 내고(설계 0026),
 *  2단계(채점)는 그 지시문의 모호성을 질문으로 좁혀 점수를 확정한다(설계 0013).
 *  상태는 단계별 리듀서 둘이 그대로 갖고, 이 패널은 어느 단계를 보여줄지만 고른다.
 *  진행 중에도 컴포저는 잠기지 않으며, 실패 시 수동 편집으로 폴백한다(소프트 게이트). */
export function InterviewPanel({
  grill,
  interview,
  currentInstruction,
  currentRepo,
  disabled,
  onGrillStart,
  onGrillDraft,
  onGrillAnswer,
  onGrillAcceptRecommendation,
  onGrillDontKnow,
  onGrillEndNow,
  onGrillApplyAndScore,
  onGrillApplyInstruction,
  onGrillUndoInstruction,
  onGrillSave,
  onGrillRetry,
  onInterviewStart,
  onInterviewAnswer,
  onInterviewCrystallize,
  onInterviewRetry,
  onClose,
}: Props) {
  const stage = interviewStage(grill, interview);
  const grillBusy = grill.phase === "asking" || grill.phase === "noting";
  const interviewBusy = interview.phase === "assessing" || interview.phase === "crystallizing";
  const busy = grillBusy || interviewBusy;
  // 파기 진행 중(질문 대기·답변 중·노트 정리 중) — 남은 논점과 "이만 종료"가 보이는 구간.
  const grillInProgress = grillBusy || grill.phase === "answering";
  const ambiguity = interview.result?.ambiguity ?? interview.assessment?.ambiguity ?? null;
  const stale =
    stage === "score"
      ? interviewIsStale(interview, currentInstruction, currentRepo)
      : stage === "dig"
        ? grillIsStale(grill, currentInstruction, currentRepo)
        : false;
  // 헤더 주 버튼: 채점 단계면 다시 채점, 그 밖에는 (다시) 깊게 파기.
  const onHeaderStart = stage === "score" ? onInterviewStart : onGrillStart;

  return (
    <div className="mb-2 rounded-md border border-border bg-raised/40 px-2.5 py-1.5">
      <div className="flex items-center gap-2 text-xs">
        <Icon name="sparkle" size={14} />
        <span className="text-text-secondary">인터뷰</span>
        {stage !== "idle" && <StageChips stage={stage} />}
        {stage === "dig" && (grill.phase === "asking" || grill.phase === "answering") && (
          <span className="text-text-muted">
            라운드 {Math.min(grill.transcript.length + 1, MAX_GRILL_ROUNDS)}/{MAX_GRILL_ROUNDS}
          </span>
        )}
        {ambiguity && <AmbiguityBadge ambiguity={ambiguity} />}
        <button
          type="button"
          className={`ml-auto rounded border border-border px-2 py-0.5 ${
            disabled || busy ? "text-text-muted" : "text-primary-bright hover:border-primary"
          }`}
          disabled={disabled || busy}
          onClick={onHeaderStart}
        >
          {headerLabel(stage, grill, interview)}
        </button>
        {stage === "idle" && (
          <button
            type="button"
            className={`rounded border border-border px-2 py-0.5 ${
              disabled ? "text-text-muted" : "text-text-secondary hover:text-text"
            }`}
            disabled={disabled}
            onClick={onInterviewStart}
            title="깊게 파기를 건너뛰고 지금 지시문의 모호성만 채점합니다"
          >
            채점만
          </button>
        )}
        {stage !== "idle" && onClose && (
          <IconButton size="sm" label="닫기" onClick={onClose}>
            <Icon name="x" size={14} />
          </IconButton>
        )}
      </div>

      {stale && (
        <p className="mt-1.5 text-xs text-status-awaiting">
          지시문 또는 레포가 변경되었습니다 — 인터뷰 결과가 현재 입력과 다를 수 있습니다.
          {stage === "score" && " 다시 채점하세요."}
        </p>
      )}

      {/* ── 1단계: 깊게 파기 ── */}
      {stage === "dig" && grill.phase === "answering" && grill.current && (
        <div className="mt-2">
          <GrillQuestionCard
            question={grill.current}
            draft={grill.draft}
            onDraft={onGrillDraft}
            onAnswer={onGrillAnswer}
            onAcceptRecommendation={onGrillAcceptRecommendation}
            onDontKnow={onGrillDontKnow}
          />
        </div>
      )}

      {stage === "dig" && grillInProgress && (
        <div className="flex items-end justify-between gap-2">
          <OpenThreads threads={grill.openThreads} />
          <button
            type="button"
            className={`mb-0.5 shrink-0 ${SECONDARY_BUTTON}`}
            onClick={onGrillEndNow}
          >
            이만 종료
          </button>
        </div>
      )}

      {stage === "dig" && grill.phase === "done" && grill.note && (
        <div className="mt-2 flex flex-col gap-1.5">
          {grill.forcedEnd && (
            <p className="text-xs text-status-awaiting">
              {MAX_GRILL_ROUNDS}라운드 상한에 닿아 인터뷰를 끊었습니다 — 남은 논점은 노트에
              담겼습니다. 범위를 쪼개 다시 파는 것을 권합니다.
            </p>
          )}
          <div className="flex flex-wrap items-center gap-1.5">
            {onGrillUndoInstruction ? (
              <>
                <button type="button" className={PRIMARY_BUTTON} onClick={onInterviewStart}>
                  2단계: 채점 →
                </button>
                <button type="button" className={MUTED_BUTTON} onClick={onGrillUndoInstruction}>
                  되돌리기
                </button>
              </>
            ) : (
              <>
                <button type="button" className={PRIMARY_BUTTON} onClick={onGrillApplyAndScore}>
                  지시문에 적용 → 채점
                </button>
                <button type="button" className={SECONDARY_BUTTON} onClick={onGrillApplyInstruction}>
                  적용만
                </button>
              </>
            )}
          </div>
          <GrillNoteBody grill={grill} onSave={onGrillSave} />
        </div>
      )}

      {stage === "dig" && grill.phase === "error" && (
        <div className="mt-1.5 flex items-center gap-2 text-xs">
          <span className="text-status-failed">{grill.error}</span>
          <button type="button" className={`shrink-0 ${SECONDARY_BUTTON}`} onClick={onGrillRetry}>
            재시도
          </button>
          <span className="shrink-0 text-text-muted">작업 생성은 언제든 계속할 수 있습니다.</span>
        </div>
      )}

      {/* ── 2단계: 채점 ── */}
      {stage === "score" && grill.note && (
        <details className="mt-1.5">
          <summary className="cursor-pointer text-xs text-text-muted">
            1단계 노트 — 깊게 파기 {grill.transcript.length}라운드
          </summary>
          <div className="mt-1 flex flex-col gap-1.5">
            <GrillNoteBody grill={grill} onSave={onGrillSave} />
          </div>
        </details>
      )}

      {stage === "score" && interview.phase === "answering" && interview.assessment && (
        <div className="mt-2 flex flex-col gap-1.5">
          {interview.assessment.questions.map((question) => (
            <InterviewQuestionCard
              key={question.id}
              question={question}
              answer={interview.answers[question.id] ?? ""}
              onAnswer={onInterviewAnswer}
            />
          ))}
          <button type="button" className={`self-end ${PRIMARY_BUTTON}`} onClick={onInterviewCrystallize}>
            답변 반영 — 점수 확정
          </button>
        </div>
      )}

      {stage === "score" && interview.phase === "done" && interview.result && (
        <p className="mt-1.5 text-xs text-text-muted">
          모호성 점수가 확정되었습니다 — 이 점수는 생성될 작업에 함께 기록됩니다.
        </p>
      )}

      {stage === "score" && interview.phase === "error" && (
        <div className="mt-1.5 flex items-center gap-2 text-xs">
          <span className="text-status-failed">{interview.error}</span>
          <button type="button" className={`shrink-0 ${SECONDARY_BUTTON}`} onClick={onInterviewRetry}>
            재시도
          </button>
          <span className="shrink-0 text-text-muted">수동 편집으로 계속할 수 있습니다.</span>
        </div>
      )}
    </div>
  );
}

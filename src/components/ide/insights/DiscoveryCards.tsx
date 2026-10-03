import { useState } from "react";
import {
  insightCardDismiss,
  insightMemoryAppend,
  type MemoryTarget,
  type SignalCard,
  type Task,
} from "../../../lib/ipc";
import { memoryFileOpen } from "../../../lib/memory-file-ipc";
import { projectEditorOpen } from "../../../lib/project-editor-ipc";

interface DiscoveryCardsProps {
  cards: SignalCard[];
  /** 무시된 카드를 즉시 목록에서 지운다 — 서버 재조회 없이 로컬 상태만 줄인다. */
  onDismissed: (key: string) => void;
  /** `Tasks` 근거의 각 id를 찾을 대상. 없는 id는 "찾을 수 없음"으로 대체한다. */
  tasks: Task[];
  onOpenTask: (task: Task) => void;
  /** `Spend` 근거 — 지출 섹션으로 스크롤한다(기존 스크롤 스파이 재사용). */
  onJumpToSpend: () => void;
  /** 조사 작업 시작 — 작성기를 문장으로 미리 채우되 전송하지 않는다. */
  onStartResearch: (instruction: string, repo: string | null) => void;
}

/**
 * 발견 공간 — 신호 카드(설계 2026-09-28 §5·§8, 슬라이스 T2·T4). "이번 구간에 무엇이
 * 달라졌나"를 문장으로 보여주고, 근거 열기 · 메모리에 남기기 · 조사 작업 시작 · 무시
 * 네 행동(§8)을 카드마다 제공한다.
 *
 * 카드가 없으면(집계 대상이 없거나 호출이 실패했거나) 아무것도 그리지 않는다 —
 * 이 섹션이 없어서 화면이 비어 보이는 것보다, 빈 섹션이 자리를 차지하는 쪽이 더 어색하다.
 */
export function DiscoveryCards({
  cards,
  onDismissed,
  tasks,
  onOpenTask,
  onJumpToSpend,
  onStartResearch,
}: DiscoveryCardsProps) {
  if (cards.length === 0) return null;

  return (
    <section className="pt-6" data-testid="discovery-cards">
      <div className="flex items-baseline justify-between mb-3">
        <h2 className="text-lg font-semibold">발견</h2>
      </div>
      <div className="grid gap-2.5">
        {cards.map((c) => (
          <DiscoveryCard
            key={c.key}
            card={c}
            tasks={tasks}
            onOpenTask={onOpenTask}
            onJumpToSpend={onJumpToSpend}
            onStartResearch={onStartResearch}
            onDismissed={onDismissed}
          />
        ))}
      </div>
    </section>
  );
}

function fmtBaseline(v: number): string {
  // 비율(0~1 근방, 예: S2의 "구간 평균")과 건수(예: S1·S6의 직전 구간 값)를 함께 다루므로,
  // 정수처럼 보이면 그대로, 아니면 소수 둘째 자리까지만 보여준다.
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

/** 조사 작업 프롬프트에 붙일 근거 한 줄. 원문 수치를 다시 셀 수 있게 id·번호를 그대로 남긴다. */
function evidenceSummary(card: SignalCard): string {
  switch (card.evidence.kind) {
    case "Tasks":
      return `작업 ${card.evidence.ids.map((id) => `#${id}`).join(", ")}`;
    case "Ledger":
      return `${card.evidence.repo} 원장 ${card.evidence.numbers.map((n) => `#${n}`).join(", ")}`;
    case "Spend":
      return card.evidence.project ? `프로젝트 ${card.evidence.project}` : "지출 추이";
  }
}

function researchInstruction(card: SignalCard): string {
  return `발견: ${card.sentence}\n근거: ${evidenceSummary(card)}`;
}

interface DiscoveryCardProps {
  card: SignalCard;
  tasks: Task[];
  onOpenTask: (task: Task) => void;
  onJumpToSpend: () => void;
  onStartResearch: (instruction: string, repo: string | null) => void;
  onDismissed: (key: string) => void;
}

function DiscoveryCard({
  card,
  tasks,
  onOpenTask,
  onJumpToSpend,
  onStartResearch,
  onDismissed,
}: DiscoveryCardProps) {
  const [showTasks, setShowTasks] = useState(false);
  const [showMemory, setShowMemory] = useState(false);
  const [memoryText, setMemoryText] = useState(card.sentence);
  const [memoryTarget, setMemoryTarget] = useState<"user" | "repo">(card.repo ? "repo" : "user");
  const [memoryStatus, setMemoryStatus] = useState<string | null>(null);
  const [dismissing, setDismissing] = useState(false);

  // 근거 열기 — 종류마다 다른 화면 동작을 쓴다(§8). `Tasks`만 카드 내부에 펼치고, 나머지는
  // 기존 화면 이동 수단(프로젝트 창 · 지출 섹션 스크롤)을 그대로 재사용한다.
  const openEvidence = () => {
    if (card.evidence.kind === "Spend") {
      onJumpToSpend();
      return;
    }
    if (card.evidence.kind === "Ledger") {
      // 원장 특정 항목으로 포커스를 옮기는 수단이 아직 없어, 저장소 창을 여는 데 그친다(편차).
      void projectEditorOpen(card.evidence.repo);
      return;
    }
    setShowTasks((v) => !v);
  };

  const target: MemoryTarget =
    memoryTarget === "repo" && card.repo ? { kind: "Repo", repo: card.repo } : { kind: "User" };

  const saveMemory = async () => {
    setMemoryStatus(null);
    const outcome = await insightMemoryAppend(target, memoryText);
    if (outcome.kind === "Full") {
      setMemoryStatus(`상한을 넘어 자동으로 남기지 못했습니다 — 연 파일에서 직접 정리해 주세요.`);
      void memoryFileOpen(outcome.path);
      return;
    }
    setMemoryStatus("메모리에 남겼습니다.");
    setShowMemory(false);
  };

  const dismiss = async () => {
    setDismissing(true);
    try {
      await insightCardDismiss(card.signal, card.repo);
      onDismissed(card.key);
    } finally {
      setDismissing(false);
    }
  };

  return (
    <div className="bg-surface border border-border rounded-lg p-3">
      <div className="text-sm">{card.sentence}</div>
      <div className="text-xs text-text-secondary mt-1 font-code">
        표본 {card.sample}
        {card.baseline != null && ` · 기준 ${fmtBaseline(card.baseline)}`}
      </div>

      {showTasks && card.evidence.kind === "Tasks" && (
        <ul className="mt-2 text-xs border-t border-border pt-2 grid gap-1">
          {card.evidence.ids.map((id) => {
            const task = tasks.find((t) => t.id === id);
            return (
              <li key={id}>
                <button
                  className="text-primary-bright hover:underline disabled:text-text-muted disabled:no-underline"
                  disabled={!task}
                  onClick={() => task && onOpenTask(task)}
                >
                  #{id} {task ? task.instruction.slice(0, 60) : "(찾을 수 없음)"}
                </button>
              </li>
            );
          })}
        </ul>
      )}

      <div className="flex items-center gap-1 mt-2 text-xs">
        <button className="px-2 h-6 rounded hover:bg-raised text-text-secondary" onClick={openEvidence}>
          근거 열기
        </button>
        <button
          className="px-2 h-6 rounded hover:bg-raised text-text-secondary"
          onClick={() => {
            setShowMemory((v) => !v);
            setMemoryStatus(null);
          }}
        >
          메모리에 남기기
        </button>
        <button
          className="px-2 h-6 rounded hover:bg-raised text-text-secondary"
          onClick={() => onStartResearch(researchInstruction(card), card.repo)}
        >
          조사 작업 시작
        </button>
        <button
          className="px-2 h-6 rounded hover:bg-raised text-text-secondary ml-auto disabled:opacity-40"
          onClick={() => void dismiss()}
          disabled={dismissing}
        >
          무시
        </button>
      </div>

      {showMemory && (
        <div className="mt-2 border-t border-border pt-2">
          <textarea
            className="w-full text-xs bg-bg border border-border rounded p-1.5 font-ui"
            rows={2}
            value={memoryText}
            onChange={(e) => setMemoryText(e.target.value)}
          />
          <div className="flex items-center gap-2 mt-1.5 text-xs">
            {card.repo && (
              <div className="flex items-center gap-1 bg-bg border border-border rounded p-0.5">
                <button
                  className={`px-1.5 h-5 rounded ${
                    memoryTarget === "user" ? "bg-raised text-text" : "text-text-secondary"
                  }`}
                  onClick={() => setMemoryTarget("user")}
                >
                  USER.md
                </button>
                <button
                  className={`px-1.5 h-5 rounded ${
                    memoryTarget === "repo" ? "bg-raised text-text" : "text-text-secondary"
                  }`}
                  onClick={() => setMemoryTarget("repo")}
                >
                  {card.repo}/MEMORY.md
                </button>
              </div>
            )}
            <button
              className="px-2 h-6 rounded bg-primary/15 text-primary-bright ml-auto"
              onClick={() => void saveMemory()}
            >
              저장
            </button>
          </div>
          {memoryStatus && <div className="text-text-secondary mt-1">{memoryStatus}</div>}
        </div>
      )}
    </div>
  );
}

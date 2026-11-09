import { canvasProgress, parseCanvasNodes, type CanvasStatus } from "../../lib/canvas";
import type { Capsule } from "../../lib/ipc";
import { Icon } from "./icons";
import { IconButton } from "../ui";

const STATUS_MARK: Record<CanvasStatus, string> = { done: "●", doing: "◐", todo: "○" };
const STATUS_TONE: Record<CanvasStatus, string> = {
  done: "text-status-done",
  doing: "text-status-running",
  todo: "text-text-muted",
};

/**
 * 에이전트가 선언한 계획을 노드 목록으로. Mermaid 런타임은 넣지 않는다(계획 0033 DR-5) —
 * 캔버스의 가치는 에이전트가 읽는 기호 밀도이고, 사람에게는 목록이 더 읽기 쉽다.
 */
export function CanvasSection({ canvas }: { canvas: string }) {
  const nodes = parseCanvasNodes(canvas);
  const progress = canvasProgress(nodes);
  if (!progress) return null;
  return (
    <div className="mt-2 pt-2 border-t border-border">
      <div className="text-xs text-text-muted mb-1">
        작업 캔버스 · {progress.done}/{progress.total}
      </div>
      <div className="flex flex-col gap-0.5">
        {nodes.map((n) => (
          <div key={n.id} className="flex items-baseline gap-1.5 text-xs">
            <span className={STATUS_TONE[n.status]}>{STATUS_MARK[n.status]}</span>
            <span
              className={`min-w-0 flex-1 truncate ${
                n.status === "done" ? "text-text-muted" : "text-text-secondary"
              }`}
              title={n.summary}
            >
              {n.summary}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

interface Props {
  capsule: Capsule;
  onForward: (nextAction: string) => void; // 입력창 프리필 (라이브 에이전트)
  onInject: () => void; // AGENTS.md 주입 (다음 세션)
  onClose: () => void;
}

/** 작업 캡슐 브리핑 패널 — nextAction + 상태/변경/증거/미해결/최근. */
export function CapsulePanel({ capsule, onForward, onInject, onClose }: Props) {
  const c = capsule;
  return (
    <div className="absolute right-4 bottom-20 z-30 w-[30rem] max-h-80 overflow-auto rounded-lg border border-border-strong bg-raised shadow-e2 p-3">
      <div className="flex items-center justify-between mb-2">
        <span className="text-xs uppercase tracking-wide text-text-muted">Capsule · {c.state}</span>
        <IconButton size="sm" label="닫기" onClick={onClose}>
          <Icon name="x" size={14} />
        </IconButton>
      </div>

      <div className="mb-2 p-2 rounded bg-bg border border-border">
        <div className="text-xs text-text-muted mb-0.5">다음 행동</div>
        <div className="text-sm text-text">{c.next_action}</div>
      </div>

      <div
        className="text-sm text-text-secondary mb-1 truncate"
        title={c.goal_contract?.objective ?? c.instruction}
      >
        <span className="text-text-muted">Goal: </span>
        {c.goal_contract?.objective ?? c.instruction}
      </div>
      {c.goal_contract && c.goal_contract.acceptance.length > 0 && (
        <div className="mb-1 text-xs text-status-awaiting">
          완료 기준 {c.goal_contract.acceptance.length}개 · 수동 확인 필요
        </div>
      )}
      <div className="text-xs text-text-muted mb-1">
        브랜치 <span className="font-code">{c.branch}</span> · 변경 {c.changed.length}개
        {c.evidence_summary && <> · 검증 {c.evidence_summary}</>}
      </div>
      <CanvasSection canvas={c.canvas} />

      {c.recent.length > 0 && (
        <div className="mt-2 pt-2 border-t border-border">
          <div className="text-xs text-text-muted mb-1">최근</div>
          <div className="flex flex-col gap-0.5">
            {c.recent.map((r, i) => (
              <div key={`${i}-${r}`} className="text-xs font-code text-text-secondary truncate">
                {r}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="flex gap-2 mt-2 pt-2 border-t border-border">
        <button
          className="h-7 px-2 rounded-md bg-primary text-bg text-sm"
          onClick={() => onForward(c.next_action)}
          title="다음 행동을 입력창에 채웁니다 (자동 전송 아님)"
        >
          현재 에이전트에 전달
        </button>
        <button
          className="h-7 px-2 rounded-md border border-border text-text-secondary text-sm hover:border-border-strong"
          onClick={onInject}
          title="AGENTS.md에 브리핑 저장 — 다음 세션이 읽습니다"
        >
          파일에 저장
        </button>
        <button
          className="h-7 px-2 rounded-md text-text-secondary text-sm hover:text-text"
          onClick={onClose}
        >
          닫기
        </button>
      </div>
    </div>
  );
}

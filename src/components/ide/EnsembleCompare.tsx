import type { DiffHunk, HunkRef, Task } from "../../lib/ipc";
import { hunkKey } from "../../lib/ensemble-compose";
import { hunkLines } from "../../lib/annotations";
import { useEnsembleCompare } from "./use-ensemble-compare";
import { Icon } from "./icons";

interface Props {
  candidates: Task[];
  winnerTaskId: number;
  ensemble: string;
  /** 조합 병합 성공 후 호출 — 부모가 후보 목록/diff를 새로고침하고 verify 트리거를 이어간다. */
  onComposed: () => void;
}

function candidateLabel(t: Task): string {
  return t.agent ?? t.branch;
}

/** hunk 한 칸 — 체크박스(protected/베이스는 비활성+사유 툴팁) + 헤더 + 라인. */
function HunkPanelItem({
  hunk,
  checked,
  disabled,
  disabledReason,
  onToggle,
}: {
  hunk: DiffHunk;
  checked: boolean;
  disabled: boolean;
  disabledReason?: string;
  onToggle: () => void;
}) {
  return (
    <div className="mb-2 rounded border border-border overflow-hidden">
      <div className="px-2 py-1 flex items-center gap-2 text-primary-bright bg-empty text-xs font-code">
        <input type="checkbox" checked={checked} disabled={disabled} title={disabledReason} onChange={onToggle} />
        <span>
          @@ -{hunk.old_range[0]},{hunk.old_range[1]} +{hunk.new_range[0]},{hunk.new_range[1]} @@
        </span>
        {hunk.protected && <span className="ml-auto text-status-failed">protected</span>}
      </div>
      <div className="text-xs font-code leading-relaxed">
        {hunkLines(hunk).map((line, i) => (
          <div
            key={i}
            className={`px-2 whitespace-pre-wrap ${
              line.kind === "add"
                ? "bg-addbg text-status-done"
                : line.kind === "del"
                  ? "bg-delbg text-status-failed"
                  : "text-text-secondary"
            }`}
          >
            {(line.kind === "add" ? "+" : line.kind === "del" ? "-" : " ") + line.text}
          </div>
        ))}
      </div>
    </div>
  );
}

/** B-3 ensemble 조합 병합 — 후보 탭(심판 추천 배지), 파일 셀렉터, side-by-side hunk 패널.
 *  겹치는 hunk(배타 그룹)는 그룹당 최대 1개만 선택 가능(백엔드 `ensemble::compose` 이중 방어와 대칭). */
export function EnsembleCompare({ candidates, winnerTaskId, ensemble, onComposed }: Props) {
  const s = useEnsembleCompare(ensemble, candidates, winnerTaskId, onComposed);
  const winner = candidates.find((c) => c.id === winnerTaskId);
  const active = candidates.find((c) => c.id === s.activeId);
  const winnerHunks = (s.hunksByTask[winnerTaskId] ?? []).filter((h) => h.path === s.file);
  const activeHunks = (s.hunksByTask[s.activeId] ?? []).filter((h) => h.path === s.file);

  if (candidates.length < 2) {
    return (
      <div className="flex-1 flex items-center justify-center text-text-muted text-sm">
        후보가 2개 이상일 때 조합 비교를 사용할 수 있습니다.
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col min-h-0">
      <div className="h-9 shrink-0 border-b border-border flex items-center gap-1 px-2 overflow-x-auto">
        {candidates.map((c) => (
          <button
            key={c.id}
            className={`text-xs px-2 py-1 rounded flex items-center gap-1 shrink-0 ${
              s.activeId === c.id ? "bg-raised text-text" : "text-text-secondary hover:text-text"
            }`}
            onClick={() => s.setActiveId(c.id)}
          >
            <span className="font-code">{candidateLabel(c)}</span>
            {c.id === winnerTaskId && (
              <span className="text-[11px] px-1 rounded bg-primary/20 text-primary-bright">심판 추천</span>
            )}
          </button>
        ))}
        <select
          className="ml-auto text-xs bg-bg border border-border rounded px-1.5 py-1 text-text-secondary outline-none focus:border-primary shrink-0"
          value={s.file ?? ""}
          onChange={(e) => s.setFile(e.target.value)}
        >
          {s.allPaths.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      </div>

      {s.loadError && (
        <div className="bg-dangerbg border-b border-dangerborder text-status-failed text-sm px-3 py-1 font-code shrink-0">
          {s.loadError}
        </div>
      )}
      {s.notice && (
        <div className="px-3 py-1 text-xs shrink-0 flex items-center gap-1" style={{ color: "var(--c-awaiting)" }}>
          <Icon name="alert" size={12} />{s.notice}
        </div>
      )}

      <div className="flex-1 min-h-0 overflow-auto grid grid-cols-2 gap-3 p-3">
        <div>
          <div className="text-xs text-text-muted mb-1 font-code">{winner ? candidateLabel(winner) : ""} (베이스)</div>
          {winnerHunks.map((h) => (
            <HunkPanelItem
              key={h.id}
              hunk={h}
              checked={s.selection.has(hunkKey({ task_id: winnerTaskId, hunk_id: h.id }))}
              disabled
              disabledReason="심판 추천 후보의 베이스 — 항상 포함됩니다"
              onToggle={() => {}}
            />
          ))}
        </div>
        {s.activeId !== winnerTaskId && (
          <div>
            <div className="text-xs text-text-muted mb-1 font-code">{active ? candidateLabel(active) : ""}</div>
            {activeHunks.map((h) => {
              const ref: HunkRef = { task_id: s.activeId, hunk_id: h.id };
              return (
                <HunkPanelItem
                  key={h.id}
                  hunk={h}
                  checked={s.selection.has(hunkKey(ref))}
                  disabled={h.protected}
                  disabledReason={h.protected ? "protected 경로 변경 — 조합 병합으로 유입할 수 없습니다" : undefined}
                  onToggle={() => s.toggle(ref)}
                />
              );
            })}
          </div>
        )}
      </div>

      <footer className="shrink-0 border-t border-border px-3 py-2 flex items-center gap-3 text-xs">
        <span className="text-text-secondary">
          선택: {Object.entries(s.summary).map(([id, n]) => `${id}:${n}`).join(" · ") || "없음"}
        </span>
        {s.appliedCount != null && (
          <span className="text-status-done">조합 적용 완료 — {s.appliedCount}개 hunk</span>
        )}
        <button
          className="ml-auto flex items-center gap-1 text-sm px-3 py-1 rounded-md bg-primary/15 text-primary-bright disabled:opacity-40 shrink-0"
          disabled={s.busy || s.composeArgs.length === 0}
          onClick={() => void s.compose()}
          title="타 후보의 선택 hunk를 심판 추천 후보에 적용하고 verify를 트리거합니다"
        >
          <Icon name="scale" size={14} />
          {s.busy ? "조합 병합 중…" : "조합 병합 → verify"}
        </button>
      </footer>
    </div>
  );
}

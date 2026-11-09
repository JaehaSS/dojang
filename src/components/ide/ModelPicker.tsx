import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./icons";
import { modelsForAgentWithObserved, type ModelOption } from "../../lib/models";
import { useObservedModels } from "../../lib/use-observed-models";
import { useAvailableModels } from "../../lib/use-available-models";
import { agentModelsGet } from "../../lib/ipc";
import { filterByQuery, queryTokens } from "./picker-search";
import { Highlighted } from "./PickerHighlight";
import { usePickerCursor } from "./usePickerCursor";

interface Props {
  /** 단일 선택된 벤더 key — 앙상블(2개+)에서는 이 컴포넌트를 렌더하지 않는다. */
  agent: string;
  /** 이 세션에만 적용할 모델 오버라이드 ("" = 설정의 벤더 기본). */
  model: string;
  /** 마지막으로 **관측된** 실행 모델(`model_snapshot`의 resolved). 있으면 칩은 이것을 보여준다 —
   *  요청값은 CLI가 별칭을 어떻게 풀었는지도(`opus` → `claude-opus-5[1m]`), 전환이 아직
   *  반영되지 않았다는 것도 말해주지 못한다. 관측이 없는 자리(컴포저)는 넘기지 않는다. */
  observedModel?: string | null;
  onChange: (model: string) => void;
  /** 팝오버가 열리는 방향. 기본 "up"은 화면 하단 컴포저용이고, 상단 헤더에 놓을 때는
   *  "down"이어야 한다 — 위로 열면 44px 헤더 밖으로 잘려 항목을 고를 수 없다. */
  placement?: "up" | "down";
  /** 고를 수 없는 자리 — 토론처럼 두 면이 이미 벤더 세션을 쥐고 있을 때. */
  disabled?: boolean;
  /** disabled일 때 칩 툴팁을 대신할 이유. */
  disabledTitle?: string;
}

/**
 * 칩 툴팁. 라벨이 관측값으로 갈리므로 요청값과 어긋나는 경우를 여기서 밝힌다.
 *
 * 관측이 없는 상태를 확인된 것처럼 보이게 두지 않는 것이 이 함수의 요점이다 — 전환은 다음
 * resume 턴부터 실리고, codex는 턴이 끝나야 관측값이 온다.
 */
export function modelChipTitle({
  requested,
  running,
  vendorDefault,
}: {
  requested: string;
  running: string;
  vendorDefault: string;
}): string {
  const origin = requested
    ? `이 세션의 모델: ${requested}`
    : `모델: 벤더 기본값${vendorDefault ? ` (${vendorDefault})` : " (CLI 기본)"} — 설정 > 에이전트 모델`;
  if (!running) return requested ? `${origin} — 아직 실행으로 확인되지 않았습니다` : origin;
  if (running === requested) return `실행 중: ${running} (이 세션에 지정)`;
  return `실행 중: ${running}\n${origin}`;
}

export interface ModelRow {
  kind: "default" | "model" | "custom" | "free";
  /** 확정하면 `onChange`에 넘길 값. */
  value: string;
  label: string;
  /** 오른쪽 꼬리표 — 모델 id, 벤더 기본값, "커스텀". */
  trailing?: string;
  /** 질의로 걸러진 행만 강조한다 — 기본·직접 지정은 매치가 아니다. */
  match?: boolean;
}

/**
 * 메뉴 행 — `[기본 (설정값)] + 매치 + (카탈로그 밖 현재값이면) 커스텀 + 직접 지정`.
 *
 * "기본"은 질의와 무관하게 늘 최상단이고, `직접 지정: '<질의>'`는 질의가 어떤 후보 id와도
 * 정확히 같지 않으면 **매치 수와 무관하게** 맨 아래에 있다. 0건 전용으로 두면 카탈로그의
 * 접두 관계(`gemini-3.6-flash` → `-high/-medium/-low`) 때문에 Enter가 다른 모델을 조용히
 * 고른다(ADR 0176).
 */
export function modelPickerRows(
  options: ModelOption[],
  model: string,
  query: string,
  vendorDefault: string,
): ModelRow[] {
  const q = query.trim();
  const rows: ModelRow[] = [
    { kind: "default", value: "", label: "기본 (설정값)", trailing: vendorDefault || undefined },
  ];
  for (const o of filterByQuery(options, query, (m) => `${m.label ?? m.id} ${m.id}`)) {
    rows.push({ kind: "model", value: o.id, label: o.label ?? o.id, trailing: o.id, match: true });
  }
  const current = model.trim();
  const custom = current.length > 0 && !options.some((o) => o.id === current);
  // 커스텀 행을 누르면 기본으로 되돌린다 — 지금 값이므로 다시 고를 것이 없다.
  if (custom && filterByQuery([current], query, (id) => id).length > 0) {
    rows.push({ kind: "custom", value: "", label: current, trailing: "커스텀", match: true });
  }
  const exact = options.some((o) => o.id === q) || current === q;
  if (q.length > 0 && !exact) rows.push({ kind: "free", value: q, label: `직접 지정: '${q}'` });
  return rows;
}

/** 세션 단위 모델 선택 칩 — 이번 작업에만 적용되는 오버라이드.
 *  비워두면 설정 > 에이전트 모델의 벤더 기본값(없으면 CLI 기본)을 따른다.
 *  카탈로그(models.ts)는 후보 제안일 뿐이며 커스텀 자유입력이 항상 폴백.
 *
 *  **고르는 값과 보여주는 값이 다르다.** 드롭다운은 요청값(`model`)을 고치고, 칩 라벨은
 *  `observedModel`이 있으면 그것을 보여준다. 요청값만 보여주던 시절 칩은 세 자리에서 사실과
 *  달랐다 — 기본값일 때 무엇이 도는지 말하지 못했고, 전환 직후 아직 실리지도 않은 모델을 걸었고,
 *  별칭(`opus`)이 무엇으로 풀렸는지 감췄다. */
export function ModelPicker({
  agent,
  model,
  observedModel,
  onChange,
  placement = "up",
  disabled = false,
  disabledTitle,
}: Props) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  // 설정의 벤더 기본 모델 — "기본" 항목에 실제 값을 보여주기 위한 조회(실패해도 무시).
  const [defaults, setDefaults] = useState<Record<string, string>>({});
  // CLI 조회 목록에 없는 모델도 한 번 쓴 적 있으면 후보로 올린다.
  const observed = useObservedModels();
  const available = useAvailableModels(agent, open);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    agentModelsGet()
      .then(setDefaults)
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const options = modelsForAgentWithObserved(agent, observed, available);
  const vendorDefault = (defaults[agent] ?? "").trim();
  const labelOf = (id: string) => options.find((o) => o.id === id)?.label ?? id;
  const requested = model.trim();
  // 드롭다운의 체크 표시는 계속 **요청값** 기준이다 — 거기서 고르는 것이 그 값이기 때문이다.
  // 칩 라벨만 관측값으로 갈린다.
  const overridden = requested.length > 0;
  const running = (observedModel ?? "").trim();
  // 관측이 있으면 그것이 칩의 진실. 없으면(전환 직후·첫 턴·codex의 진행 중 턴) 요청값으로 돌아간다.
  const shown = running || requested;
  const chipLabel = shown ? labelOf(shown) : "모델: 기본";
  const chipTitle = modelChipTitle({ requested, running, vendorDefault });

  const tokens = useMemo(() => queryTokens(query), [query]);
  const rows = modelPickerRows(options, model, query, vendorDefault);
  const matches = rows.filter((r) => r.match).length;

  const pick = (id: string) => {
    onChange(id);
    setOpen(false);
  };

  const isOn = (row: ModelRow) =>
    row.kind === "default" ? !overridden : row.kind === "custom" ? true : model === row.value;

  // 질의가 없으면 지금 고른 행, 있으면 첫 매치(0건이면 직접 지정 행 — 둘 다 인덱스 1이다).
  const selectedRow = rows.findIndex(isOn);
  const { cursor, setCursor, inputRef, listRef, onKeyDown } = usePickerCursor({
    open,
    count: rows.length,
    initial: tokens.length > 0 ? 1 : Math.max(selectedRow, 0),
    resetOn: "open+query",
    // 비동기 조회로 행이 바뀌면 이전 인덱스로 다른 모델을 확정하지 않도록 커서를 다시 맞춘다.
    query: JSON.stringify([query, rows.map((row) => [row.kind, row.value])]),
    onCommit: (i) => {
      const row = rows[i];
      if (row) pick(row.value);
    },
    onClose: () => setOpen(false),
  });

  useEffect(() => {
    if (!open) setQuery("");
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        className={`flex items-center gap-1 text-xs border rounded-md px-2 py-1 hover:border-border-strong disabled:cursor-not-allowed disabled:opacity-60 ${
          overridden ? "text-primary-bright border-primary/50" : "text-text-secondary border-border"
        }`}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title={disabled ? disabledTitle ?? chipTitle : chipTitle}
      >
        <Icon name="sparkle" size={14} />
        <span className="max-w-[150px] truncate">{chipLabel}</span>
        <Icon name="chevronDown" size={12} />
      </button>

      {open && (
        <div
          className={`absolute left-0 z-20 w-64 rounded-lg border border-border-strong bg-raised py-1 shadow-e2 ${
            placement === "down" ? "top-full mt-1" : "bottom-full mb-1"
          }`}
        >
          <div className="flex items-center justify-between gap-2 px-3 py-1">
            <span className="text-xs text-text-muted">
              모델 <span className="text-text-secondary">— 이 세션에만 적용</span>
            </span>
            {/* 좁혀졌다는 사실을 숫자로 준다 — 스크롤 막대 길이는 근거가 아니다. */}
            <span className="text-xs text-text-muted font-code shrink-0">
              {tokens.length > 0 ? `${matches}/${options.length}` : options.length}
            </span>
          </div>

          {/* 검색 필드가 곧 자유 입력 필드다 — 64폭 팝오버에 텍스트 필드를 둘 두지 않는다(ADR 0176). */}
          <div className="px-2 pb-1">
            <div className="relative">
              <span className="absolute left-2 top-1/2 -translate-y-1/2 text-text-muted pointer-events-none">
                <Icon name="search" size={12} />
              </span>
              <input
                ref={inputRef}
                className="w-full bg-bg border border-border rounded pl-7 py-1 text-sm text-text outline-none focus:border-primary placeholder:text-text-muted"
                placeholder="검색 또는 모델 ID 입력"
                value={query}
                role="combobox"
                aria-expanded
                aria-controls="model-picker-list"
                aria-activedescendant={rows[cursor] ? `model-opt-${cursor}` : undefined}
                aria-label="모델 검색"
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={onKeyDown}
              />
            </div>
          </div>

          <div
            ref={listRef}
            id="model-picker-list"
            role="listbox"
            aria-label="모델"
            className={placement === "down" ? "max-h-64 overflow-auto" : "max-h-72 overflow-auto"}
          >
            {rows.map((row, i) => {
              const on = isOn(row);
              // 매치 0건이면 왜 없는지 말하고, 그 아래 직접 지정 행이 다음 수를 준다.
              const empty = row.kind === "free" && matches === 0;
              return (
                <div key={`${row.kind}-${row.label}`}>
                  {empty && (
                    <div className="px-3 py-2 text-sm text-text-muted">
                      '{query.trim()}'와 일치하는 모델이 없습니다
                    </div>
                  )}
                  <button
                    id={`model-opt-${i}`}
                    role="option"
                    aria-selected={on}
                    data-cursor={i === cursor ? "true" : undefined}
                    className={`w-full text-left flex items-center gap-2 px-2 py-1.5 text-sm ${
                      i === cursor ? "bg-surface text-text" : on ? "text-text" : "text-text-secondary"
                    }`}
                    onMouseEnter={() => setCursor(i)}
                    onClick={() => pick(row.value)}
                    title={
                      row.kind === "default"
                        ? "설정 > 에이전트 모델의 벤더 기본값을 따름"
                        : row.kind === "custom"
                          ? "클릭하면 기본값으로 되돌림"
                          : undefined
                    }
                  >
                    <span className={`shrink-0 w-4 ${on ? "text-primary-bright" : "text-text-muted"}`}>
                      <Icon name={on ? "check" : row.kind === "free" ? "chevronRight" : "sparkle"} size={14} />
                    </span>
                    <span className={`truncate flex-1 ${row.kind === "custom" ? "font-code text-xs" : ""}`}>
                      {row.match ? <Highlighted text={row.label} tokens={tokens} /> : row.label}
                    </span>
                    {row.trailing && (
                      <span className="text-text-muted text-xs font-code shrink-0 max-w-[110px] truncate">
                        {row.kind === "model" ? (
                          <Highlighted text={row.trailing} tokens={tokens} />
                        ) : (
                          row.trailing
                        )}
                      </span>
                    )}
                  </button>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

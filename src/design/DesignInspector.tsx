import type { DesignSelection } from "../lib/design-canvas/selection";
import type { DesignTokens } from "./token-match";
import { createTokenNormalizer, matchTokens } from "./token-match";
import type { useDesignEdits } from "./useDesignEdits";
import { useState } from "react";

type Match = { names: string[]; outside: boolean };
export function selectionTokenMatches(selection: DesignSelection, tokens: DesignTokens, document: Document, context?: Element | null): Record<string, Match> {
  const normalizer = createTokenNormalizer(document, context);
  try {
    return Object.fromEntries(Object.entries(selection.computedCss).map(([property, value]) => [property, matchTokens(property, value, tokens, normalizer.normalize)]));
  } finally { normalizer.dispose(); }
}

function TokenNote({ match }: { match?: Match }) {
  if (!match) return null;
  return match.names.length ? <span className="block break-words text-xs text-primary-bright">{match.names.join(" · ")}</span>
    : match.outside ? <span className="block text-xs text-status-awaiting">토큰 밖 값</span> : null;
}

function SpaceBox({ selection, matches, kind }: { selection: DesignSelection; matches: Record<string, Match>; kind: "margin" | "padding" }) {
  const sides = ["top", "right", "bottom", "left"];
  return <div className="mt-2 rounded border border-border p-2" aria-label={`${kind} 박스`}>
    <p className="mb-1 text-xs text-text-secondary">{kind}</p>
    <div className="grid grid-cols-2 gap-1 text-xs">
      {sides.map((side) => <div key={side} className="min-w-0 rounded border border-border px-1 py-1">
        <span className="text-text-secondary">{side}: </span>{selection.computedCss[`${kind}-${side}`] ?? "없음"}<TokenNote match={matches[`${kind}-${side}`]} />
      </div>)}
    </div>
  </div>;
}

export function DesignInspector({ selection, matches, onSelectAncestor, edit, inlineProperties = [] }: { selection: DesignSelection | null; matches: Record<string, Match>; onSelectAncestor: () => void; edit?: ReturnType<typeof useDesignEdits>; inlineProperties?: string[] }) {
  const [agentPrompt, setAgentPrompt] = useState(false);
  const properties = [["모서리", "border-radius"], ["배경", "background-color"], ["글자색", "color"], ["글꼴", "font-family"], ["글꼴 크기", "font-size"], ["글꼴 굵기", "font-weight"]];
  return <aside className="overflow-auto border-l border-border bg-surface p-3" aria-label="인스펙터">
    <h2 className="mb-3 font-medium">인스펙터</h2>
    {!selection ? <p className="text-text-secondary">캔버스에서 요소를 선택하면 정보가 표시됩니다.</p> : <>
      <p className="font-medium">{selection.tagName}</p>
      <p className="mt-1 break-all font-code text-xs text-text-secondary" aria-label="소스 위치">{selection.source ?? "소스 태그가 없습니다."}</p>
      <p role="status" className="mt-2 rounded border border-border px-2 py-1 text-xs text-text-secondary">
        {edit?.info?.editable ? "소스에 직접 기록" : (edit?.info?.kind ?? selection.classKind) === "dynamic" ? "동적 className · 읽기 전용" : "읽기 전용"}
      </p>
      {(edit?.info?.kind ?? selection.classKind) === "dynamic" && !edit?.info?.editable && <p className="mt-1 text-xs text-text-secondary">대화에서 에이전트에게 변경을 요청하세요.</p>}
      {(edit?.info?.kind ?? selection.classKind) === "dynamic" && !edit?.info?.editable && <button type="button" onClick={() => setAgentPrompt((value) => !value)} className="mt-2 rounded border border-border px-2 py-1 text-xs">에이전트에게 요청</button>}
      {agentPrompt && (edit?.info?.kind ?? selection.classKind) === "dynamic" && <div className="mt-2 text-xs text-text-secondary">
        <p>아래 요청문을 복사하여 현재 작업 대화에 붙여 넣고 원하는 변경을 적으세요.</p>
        <textarea aria-label="에이전트 요청문" readOnly onFocus={(event) => event.target.select()} value={`디자인 캔버스에서 선택한 ${selection.source ?? "태그 없는 요소"}의 동적 className을 수정해 주세요. 현재 className: ${selection.className}. 원하는 변경: `} className="mt-1 h-28 w-full rounded border border-border bg-raised p-2 text-text" />
      </div>}
      {edit?.enabled && (selection.instanceSource || edit.info?.shared) && <label className="mt-3 block text-xs">수정 범위
        <select aria-label="수정 범위" value={edit.scope} disabled={edit.status === "writing"} onChange={(event) => void edit.chooseScope(event.target.value as "instance" | "component")} className="mt-1 w-full rounded border border-border bg-raised p-1">
          <option value="instance">현재 사용처</option><option value="component">컴포넌트 정의 · 모든 사용처</option>
        </select>
      </label>}
      {edit?.enabled && edit.info?.reason && <p className="mt-2 text-xs text-status-awaiting">{edit.info.reason === "no_classname_prop" ? "이 컴포넌트는 className을 받지 않아 현재 사용처를 편집할 수 없습니다." : edit.info.reason === "no_classname_target" ? "className이 선택한 요소에 전달되지 않아 현재 사용처를 편집할 수 없습니다." : edit.info.reason === "instance_source_missing" ? "사용처 위치를 확인할 수 없어 현재 사용처를 편집할 수 없습니다." : edit.info.reason}</p>}
      {edit?.enabled && edit.status !== "idle" && <p role="status" className="mt-2 text-xs">{({ inspecting: "편집 가능 여부 확인 중", writing: "소스 기록 중", "waiting-hmr": "기록 완료 · 화면 갱신 대기", "write-failed": "write-failed · 기록 실패", "stale-file": "stale-file · 파일이 변경되었습니다. 요소를 다시 선택하세요." } as Record<string, string>)[edit.status]}</p>}
      {edit?.error && <p role="alert" className="mt-2 break-words text-xs text-status-failed">{edit.error}</p>}
      {selection.suggestedAncestor && <button type="button" onClick={onSelectAncestor} className="mt-2 w-full break-words rounded border border-border px-2 py-1 text-left text-xs hover:border-primary">
        태그가 있는 가까운 조상 선택 · {selection.suggestedAncestor.tagName}
      </button>}
      <p className="mt-3 font-code text-xs" aria-label="요소 크기">W {selection.boundingRect.width.toFixed(1)} · H {selection.boundingRect.height.toFixed(1)}</p>
      <SpaceBox selection={selection} matches={matches} kind="margin" />
      <SpaceBox selection={selection} matches={matches} kind="padding" />
      <dl className="mt-3 space-y-2">
        {properties.map(([label, property]) => <div key={property}>
          <dt className="text-xs text-text-secondary">{label}</dt>
          <dd className="break-words font-code text-xs">{selection.computedCss[property] ?? "없음"}</dd>
          <TokenNote match={matches[property]} />
        </div>)}
      </dl>
      <h3 className="mt-3 text-xs text-text-secondary">className</h3>
      <pre className="mt-1 whitespace-pre-wrap break-all rounded border border-border p-2 font-code text-xs">{selection.className || "className 없음"}</pre>
      {edit?.enabled && edit.info && <div className="mt-4 space-y-2" aria-label="디자인 값 편집">
        <p className="break-all text-xs text-text-secondary">기록 대상: {edit.info.source}</p>
        {["width", "height", "padding-top", "padding-right", "padding-bottom", "padding-left", "margin-top", "margin-right", "margin-bottom", "margin-left", "border-radius", "background-color", "color", "font-size", "font-weight"].map((property) => <label key={property} className="block text-xs text-text-secondary">{property}
          {inlineProperties.includes(property) && <span className="ml-1">· 인라인 스타일 · 대화에서 변경 요청</span>}
          <input aria-label={`편집 ${property}`} type="text" disabled={!edit.info?.editable || inlineProperties.includes(property)} value={edit.drafts[property] ?? selection.computedCss[property] ?? (property === "width" ? `${selection.boundingRect.width}px` : property === "height" ? `${selection.boundingRect.height}px` : "")} onChange={(event) => edit.change(property, event.target.value)} onBlur={edit.flush} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); edit.flush(); } }} className="mt-1 block w-full rounded border border-border bg-raised px-2 py-1 font-code text-text disabled:opacity-50" />
        </label>)}
      </div>}
    </>}
  </aside>;
}

import { useEffect, useRef, useState } from "react";

import type { LspTarget } from "../../lib/ipc";
import { targetLabel } from "../../lib/lsp";

interface ReferencesPanelProps {
  embedded?: boolean;
  targets: LspTarget[];
  total?: number;
  stale?: boolean;
  onOpen: (target: LspTarget) => void;
  onClose: () => void;
  readPreview?: (target: LspTarget) => Promise<string | null>;
}

const noPreview = async () => null;

export function ReferencesPanel({ embedded = false, targets, total = targets.length, stale = false, onOpen, onClose, readPreview = noPreview }: ReferencesPanelProps) {
  const [selected, setSelected] = useState(0);
  const [preview, setPreview] = useState<string | null>(null);
  const [previewStatus, setPreviewStatus] = useState("미리보기를 불러오는 중…");
  const panelRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const requestRef = useRef(0);
  const current = targets[selected] ?? null;

  useEffect(() => {
    setSelected(0);
    (listRef.current ?? panelRef.current)?.focus();
  }, [targets]);
  useEffect(() => {
    const item = listRef.current?.querySelector<HTMLElement>(`[data-reference-index="${selected}"]`);
    item?.scrollIntoView?.({ block: "nearest" });
  }, [selected]);
  useEffect(() => {
    const request = ++requestRef.current;
    setPreview(null);
    setPreviewStatus("미리보기를 불러오는 중…");
    if (!current || current.external) return;
    void readPreview(current).then((text) => {
      if (request !== requestRef.current) return;
      setPreview(text);
      if (text == null) setPreviewStatus("이 위치의 미리보기를 제공할 수 없습니다.");
    }).catch(() => {
      if (request === requestRef.current) setPreviewStatus("미리보기를 읽지 못했습니다. 위치는 계속 열 수 있습니다.");
    });
    return () => { requestRef.current += 1; };
  }, [current, readPreview]);

  const previewLines = preview == null || current == null ? null : preview.split("\n")
    .slice(Math.max(0, current.line - 4), current.line + 3)
    .map((text, index) => `${Math.max(1, current.line - 3) + index}  ${text}`)
    .join("\n");

  const onKeyDown = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") { event.preventDefault(); onClose(); return; }
    if (event.key === "Enter" && current) { event.preventDefault(); onOpen(current); return; }
    if (event.key === "ArrowDown" && targets.length > 0) { event.preventDefault(); setSelected((value) => Math.min(value + 1, targets.length - 1)); return; }
    if (event.key === "ArrowUp" && targets.length > 0) { event.preventDefault(); setSelected((value) => Math.max(value - 1, 0)); }
  };

  return <aside ref={panelRef} className={`flex min-h-0 min-w-0 flex-col bg-raised p-3 ${embedded ? "flex-1 overflow-hidden" : "absolute right-2 top-10 z-30 max-h-[calc(100%-3rem)] w-96 max-w-[calc(100%-1rem)] rounded-lg border border-border-strong shadow-e2"}`} role="dialog" aria-label="사용처" tabIndex={-1} onKeyDown={onKeyDown}>
    <div className="mb-2 flex items-center gap-2"><h2 className="text-sm text-text">사용처 · {total}</h2><button className="ml-auto text-text-muted hover:text-text" onClick={onClose} aria-label="사용처 닫기">×</button></div>
    {targets.length === 0 ? <p className="text-xs text-text-muted">언어 서버가 사용처를 찾지 못했습니다.</p> : <div className="min-h-0 overflow-auto" ref={listRef} role="listbox" tabIndex={-1} aria-label="사용처 목록">
      {targets.map((target, index) => <button key={`${target.abs_path}:${target.line}:${target.column}`} data-reference-index={index} className={`block w-full rounded px-2 py-1 text-left ${index === selected ? "bg-bg text-text" : "text-text-muted hover:bg-bg hover:text-text"}`} role="option" aria-selected={index === selected} onClick={() => onOpen(target)} onFocus={() => setSelected(index)}><span className="block truncate font-code text-xs">{targetLabel(target)}</span></button>)}
    </div>}
    {current && !current.external && <div className="mt-2 min-h-0"><p className="text-xs text-text-muted">현재 버퍼·디스크 미리보기 · 조회한 위치와 다를 수 있음</p><pre className="mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded bg-bg p-2 font-code text-xs text-text-muted">{previewLines ?? previewStatus}</pre></div>}
    {current?.external && <p className="mt-2 text-xs text-text-muted">외부 파일 · 기본 앱에서 열기</p>}
    {stale && <p className="mt-2 text-xs text-status-awaiting">소스가 변경되었습니다. 다시 조회하세요.</p>}
    {total > targets.length && <p className="mt-2 text-xs text-text-muted">{total - targets.length}개는 표시하지 않습니다.</p>}
    <p className="mt-2 text-xs text-text-muted">언어 서버 응답 · 분석 범위에 따라 누락 가능</p>
    <p className="mt-2 text-xs text-text-muted">↑↓ 선택 · Enter 열기 · Esc 닫기</p>
  </aside>;
}

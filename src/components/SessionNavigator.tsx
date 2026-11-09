import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { Task } from "../lib/ipc";
import type { ProjectGroups } from "../lib/project-groups";
import {
  buildSessionNavigatorTree,
  filterSessionNavigatorTree,
  sessionNavigatorRows,
  type SessionNavigatorRow,
} from "../lib/session-navigator";
import { SessionNavigatorTree } from "./SessionNavigatorTree";
interface Props {
  open: boolean;
  tasks: Task[];
  projects: string[];
  groups: ProjectGroups;
  onClose: () => void;
  onOpenTask: (task: Task) => void;
}
const initiallyCollapsed = (groups: ProjectGroups): Set<string> =>
  new Set(groups.groups.filter((group) => group.collapsed).map((group) => `group:${group.id}`));

const isBranch = (row: SessionNavigatorRow): boolean =>
  row.kind === "group" || row.kind === "project";
export function SessionNavigator({ open, tasks, projects, groups, onClose, onOpenTask }: Props) {
  const inputRef = useRef<HTMLInputElement>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const rowRefs = useRef(new Map<string, HTMLElement>());
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<Set<string>>(() => initiallyCollapsed(groups));
  const [activeId, setActiveId] = useState<string | null>(null);
  const tree = useMemo(() => buildSessionNavigatorTree(projects, tasks, groups), [projects, tasks, groups]);
  const filtered = useMemo(() => filterSessionNavigatorTree(tree, query), [tree, query]);
  const searching = query.trim().length > 0;
  const rows = useMemo(
    () => sessionNavigatorRows(filtered, collapsed, searching),
    [filtered, collapsed, searching],
  );
  const selectableRows = rows.filter((row) => row.kind !== "empty");
  const active = selectableRows.find((row) => row.id === activeId) ?? selectableRows[0] ?? null;
  useEffect(() => {
    if (!open) return;
    returnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setQuery("");
    setCollapsed(initiallyCollapsed(groups));
    setActiveId(null);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [open]);
  useEffect(() => {
    if (active) rowRefs.current.get(active.id)?.scrollIntoView({ block: "nearest" });
  }, [active]);
  if (!open) return null;

  const expanded = (row: SessionNavigatorRow): boolean => searching || !collapsed.has(row.id);

  const focusRow = (row: SessionNavigatorRow): void => {
    setActiveId(row.id);
    requestAnimationFrame(() => rowRefs.current.get(row.id)?.focus());
  };

  const toggle = (row: SessionNavigatorRow): void => {
    if (!isBranch(row) || searching) return;
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(row.id)) next.delete(row.id);
      else next.add(row.id);
      return next;
    });
  };

  const close = (): void => {
    onClose();
    queueMicrotask(() => returnFocusRef.current?.focus());
  };

  const selectTask = (task: Task): void => {
    onClose();
    onOpenTask(task);
  };

  const move = (step: number, focus: boolean): void => {
    if (!active || selectableRows.length === 0) return;
    const index = selectableRows.indexOf(active);
    const target = selectableRows[(index + step + selectableRows.length) % selectableRows.length];
    if (focus) focusRow(target);
    else setActiveId(target.id);
  };

  const firstChild = (row: SessionNavigatorRow): SessionNavigatorRow | null => {
    const child = rows[rows.indexOf(row) + 1];
    return child?.depth === row.depth + 1 && child.kind !== "empty" ? child : null;
  };

  const parent = (row: SessionNavigatorRow): SessionNavigatorRow | null => {
    for (let index = rows.indexOf(row) - 1; index >= 0; index -= 1) {
      if (rows[index].depth < row.depth) return rows[index];
    }
    return null;
  };

  const activate = (row: SessionNavigatorRow): void => {
    if (row.task) selectTask(row.task);
    else toggle(row);
  };

  const onInputKeyDown = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (selectableRows.length === 0) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      move(event.key === "ArrowDown" ? 1 : -1, false);
      return;
    }
    if (event.key === "Enter" && active) {
      event.preventDefault();
      activate(active);
    }
  };

  const onTreeKeyDown = (event: KeyboardEvent<HTMLButtonElement>, row: SessionNavigatorRow): void => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      move(event.key === "ArrowDown" ? 1 : -1, true);
      return;
    }
    if (event.key === "ArrowRight") {
      const child = firstChild(row);
      if (!expanded(row)) {
        event.preventDefault();
        toggle(row);
      } else if (child) {
        event.preventDefault();
        focusRow(child);
      }
      return;
    }
    if (event.key === "ArrowLeft") {
      if (isBranch(row) && expanded(row) && !searching) {
        event.preventDefault();
        toggle(row);
      } else if (parent(row)) {
        event.preventDefault();
        focusRow(parent(row)!);
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      activate(row);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-24" onMouseDown={close}>
      <div
        className="flex max-h-[70vh] w-full max-w-xl flex-col overflow-hidden rounded-xl border border-border-strong bg-raised shadow-e3"
        role="dialog"
        aria-modal="true"
        aria-label="세션 탐색"
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.defaultPrevented) return;
          if (event.key === "Escape" && !event.nativeEvent.isComposing) {
            event.preventDefault();
            close();
          }
        }}
      >
        <div className="border-b border-border px-3 py-2">
          <input
            ref={inputRef}
            className="w-full bg-transparent text-md text-text outline-none placeholder:text-text-muted"
            placeholder="그룹·프로젝트·세션 검색…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onInputKeyDown}
            aria-label="세션 탐색 검색"
            aria-controls="session-navigator-tree"
            aria-activedescendant={active ? `session-navigator-${active.id}` : undefined}
          />
        </div>
        <div className="flex items-center justify-between border-b border-border px-3 py-1.5 text-xs text-text-muted">
          <span>↑↓ 선택 · 트리에서 ←→ · Enter 열기 · Esc 닫기</span>
          <button className="text-text-muted hover:text-text" onClick={close} aria-label="세션 탐색 닫기">닫기</button>
        </div>
        <SessionNavigatorTree rows={rows} activeId={active?.id ?? null} expanded={expanded} rowRefs={rowRefs} onKeyDown={onTreeKeyDown} onClick={(row) => { setActiveId(row.id); activate(row); }} />
      </div>
    </div>
  );
}

import { useEffect, type ReactElement } from "react";
import type { FileDiff } from "../../lib/ipc";
import { splitFilePath, summarizePatch } from "../../lib/diff";
import { isViewed, type ViewedState } from "../../lib/diff-viewed";
import { useDiffSession } from "../DiffSessionContext";
import { ChangesHeader } from "./ChangesListHeader";

const statusColor: Record<string, string> = {
  M: "text-status-running",
  A: "text-status-done",
  D: "text-status-failed",
  R: "text-primary-bright",
};

/**
 * 트리 열(208px)의 `변경` 렌즈 — 옛 Diff 화면 사이드바의 후계다.
 *
 * 세션 전체를 대상으로 하는 값들이 여기 산다: 범위·기준점·↻·주석 재전송·확인 진행률, 그리고
 * **부분 적용의 실행**. 마지막 것이 파일 하나만 보이는 탭에 있으면 보이지 않는 파일의 변경을
 * 버리게 된다(설계 DR-5).
 */
export function ChangesList() {
  const session = useDiffSession();
  const { data, viewed, toggleViewed, activeDiffPath, openDiff, retain } = session;
  // 목록이 살아 있는 동안 폴링이 돈다 — 게이트의 구독자다(설계 DR-7).
  useEffect(() => retain(), [retain]);
  // `?? null`은 타입상 군더더기지만 wire에서 온 값이라 그렇지 않다. 값이 없는데 `=== null`로만
  // 갈랐더니 `undefined`가 `.length`까지 흘러가 창 전체가 죽었다 — 이웃(DiffTab·ChangesHeader)도
  // 같은 이유로 `?? []`를 쓴다. 없으면 "아직 모른다"이고, 그 화면은 골격이다.
  const files = data.files ?? null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ChangesHeader session={session} />
      <div className="flex-1 overflow-auto py-1">
        {files === null ? (
          <Skeleton />
        ) : files.length === 0 ? (
          <p className="px-3 py-2 text-xs text-text-muted">변경 없음</p>
        ) : (
          files.map((file) => (
            <ChangeRow
              key={file.path}
              file={file}
              viewed={viewed}
              selected={activeDiffPath === file.path}
              onToggleViewed={() => toggleViewed(file.path)}
              onOpen={() => openDiff(file.path, { preview: true })}
            />
          ))
        )}
      </div>
    </div>
  );
}

/**
 * 변경 파일 수를 provider 밖으로 건넨다 — 헤더 손잡이의 배지 값이다.
 *
 * `App`은 `DiffSessionProvider`를 **자기 JSX 안에** 그리므로 자기 몸통에서는 컨텍스트를 읽을
 * 수 없다. 얇은 소비자 하나가 그 경계를 넘긴다.
 *
 * 배지도 게이트의 구독자다 — 배지는 "변경이 있다"를 알리는 유일한 손잡이인데, 목록을 한 번
 * 열어야 폴링이 시작된다면 이미 아는 사람에게만 보인다(설계 §1).
 */
export function ChangedFileCount({
  children,
}: {
  children: (count: number | null) => ReactElement;
}): ReactElement {
  const { data, retain } = useDiffSession();
  useEffect(() => retain(), [retain]);
  return children(data.files?.length ?? null);
}

function Skeleton() {
  return (
    <div aria-hidden className="space-y-2 px-3 py-2">
      {[0, 1, 2].map((row) => (
        <div key={row} className="h-3 rounded bg-border/60" />
      ))}
    </div>
  );
}

function ChangeRow({
  file,
  viewed,
  selected,
  onToggleViewed,
  onOpen,
}: {
  file: FileDiff;
  viewed: ViewedState;
  selected: boolean;
  onToggleViewed: () => void;
  onOpen: () => void;
}) {
  const summary = summarizePatch(file.patch);
  const { name, dir } = splitFilePath(file.path);
  const done = isViewed(viewed, file.path, file.patch);
  return (
    // 체크박스를 선택 버튼 *안에* 넣지 않는다 — 중첩 버튼은 클릭이 새어 파일이 바뀐다.
    <div
      className={`flex items-center gap-1.5 pl-2 pr-2 hover:bg-border ${
        selected ? "bg-border" : ""
      } ${done ? "opacity-50" : ""}`}
    >
      <input
        type="checkbox"
        className="shrink-0"
        checked={done}
        aria-label={`${file.path} 확인함`}
        title={done ? "확인함 — 클릭하면 해제" : "확인함으로 표시"}
        onChange={onToggleViewed}
      />
      <button
        className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left text-xs disabled:opacity-60"
        title={file.path}
        onClick={onOpen}
      >
        <span
          className={`w-3 shrink-0 font-code text-[11px] ${statusColor[file.status] ?? "text-text-muted"}`}
        >
          {file.status}
        </span>
        {/* 파일명을 앞에 두고 디렉터리를 뒤로 보낸다 — 208px에서 경로를 통째로 자르면
            정작 필요한 파일명이 사라진다. 자리가 모자라면 긴 쪽(디렉터리)이 먼저 줄어든다. */}
        <span className="flex min-w-0 flex-1 items-baseline gap-1 overflow-hidden">
          <span className="truncate font-code text-text-secondary">{name}</span>
          {dir && <span className="truncate font-code text-[11px] text-text-muted">{dir}</span>}
        </span>
        <span className="shrink-0 font-code text-[11px] tabular-nums">
          <span className="text-status-done">+{summary.additions}</span>{" "}
          <span className="text-status-failed">−{summary.deletions}</span>
        </span>
      </button>
    </div>
  );
}

import { useEffect, useRef, useState, type RefObject } from "react";
import { storeDiffMode, storedDiffMode } from "../../lib/diff-mode";
import { useDiffSession } from "../DiffSessionContext";
import { resolveNavigationHunk } from "../use-review-events";
import { DiffModeToggle, type DiffMode } from "../DiffPresentation";
import { DiffBody, SHORTCUT_HINT } from "../DiffViewerLayout";
import { nextFileIndex, nextHunkIndex, useDiffKeyboard } from "../use-diff-keyboard";

/** `SplitDiff`의 `min-w-[600px]`와 같은 값 — 이보다 좁으면 오른쪽 열이 잘린다(설계 DR-6). */
const MIN_SPLIT_WIDTH = 600;
const NARROW_REASON = "본문이 좁아 통합 보기로 고정됩니다 (600px 이상 필요)";

/** 잘림은 창 폭이 아니라 본문 폭의 함수다. 관측 대상이 본문 그 자체인 이유다. */
function useNarrowBody(ref: RefObject<HTMLElement | null>): boolean {
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? 0;
      setNarrow(width > 0 && width < MIN_SPLIT_WIDTH);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [ref]);
  return narrow;
}

/**
 * 에디터 탭 안의 diff 본문. 파일 하나만 그린다 — 범위·기준점·↻·부분 적용 실행 같은 세션 값은
 * 변경 목록의 몫이다(ADR 0175 결정 5).
 */
export function DiffTab({
  path,
  active = false,
  preview = false,
  onClose,
  onOpenPath,
}: {
  path: string;
  /** 포커스 그룹의 활성 탭인가 — 단축키의 소유자를 정한다(설계 DR-9). */
  active?: boolean;
  /** 이 탭이 프리뷰 자리인가 — `]`/`[`가 자리를 갈아 끼울지 새로 열지를 정한다. */
  preview?: boolean;
  onClose?: () => void;
  onOpenPath?: (path: string) => void;
}) {
  const {
    data,
    partial,
    actions,
    retain,
    toggleViewed,
    openDiff,
    setActiveDiffPath,
    navigation,
    consumeNavigation,
  } = useDiffSession();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [stored, setStored] = useState<DiffMode>(storedDiffMode);
  const narrow = useNarrowBody(scrollRef);
  const mode: DiffMode = narrow ? "unified" : stored;

  useEffect(() => retain(), [retain]);
  useEffect(() => {
    if (!active) return;
    setActiveDiffPath(path);
    return () => setActiveDiffPath(null);
  }, [active, path, setActiveDiffPath]);

  const files = data.files ?? [];
  const current = files.find((file) => file.path === path);
  const fileHunks = data.hunks.filter((hunk) => hunk.path === path);
  const annotations = data.annotations.filter((annotation) => annotation.path === path);

  // 모드 전환은 행 높이를 바꾸므로 줄 단위 앵커가 성립하지 않는다. 보던 위치의 비율을
  // 유지하는 것으로 충분하다 — 목적은 정확한 복원이 아니라 맥락 상실 방지다.
  const changeMode = (next: DiffMode) => {
    if (narrow) return;
    const node = scrollRef.current;
    const range = node ? node.scrollHeight - node.clientHeight : 0;
    const ratio = node && range > 0 ? node.scrollTop / range : 0;
    setStored(next);
    storeDiffMode(next);
    requestAnimationFrame(() => {
      const after = scrollRef.current;
      if (!after) return;
      after.scrollTop = ratio * (after.scrollHeight - after.clientHeight);
    });
  };

  // 에이전트의 `review_navigate`. 목표 hunk의 헤더가 그려진 뒤에만 스크롤하고, 그 다음에
  // 지운다 — 탭을 막 연 참이면 첫 렌더에는 hunk가 아직 없다. 다른 파일의 목표는 건드리지 않는다.
  // id가 화면과 어긋나면 줄 번호로 되찾고, 이 파일의 hunk가 그려졌는데도 못 찾으면 버린다 —
  // 남겨 두면 범위를 바꾸는 순간 묵은 목표로 튄다.
  useEffect(() => {
    if (!navigation || navigation.path !== path) return;
    const hunkId = resolveNavigationHunk(navigation, fileHunks);
    const header = [...(scrollRef.current?.querySelectorAll<HTMLElement>("[data-hunk-id]") ?? [])]
      .find((node) => node.dataset.hunkId === hunkId);
    if (header) header.scrollIntoView({ block: "start" });
    else if (fileHunks.length === 0 && data.files === null) return;
    consumeNavigation();
  }, [navigation, path, fileHunks, data.files, consumeNavigation]);

  // 두 렌더러 모두 hunk 헤더에 data-hunk-id를 단다. 좌표 계산은 nextHunkIndex가 맡는다.
  const jumpHunk = (delta: number) => {
    const container = scrollRef.current;
    if (!container) return;
    const headers = [...container.querySelectorAll<HTMLElement>("[data-hunk-id]")];
    const tops = headers.map((header) => header.getBoundingClientRect().top);
    const target = nextHunkIndex(tops, container.getBoundingClientRect().top, delta);
    if (target >= 0) headers[target].scrollIntoView({ block: "start" });
  };

  const moveFile = (delta: number) => {
    const index = files.findIndex((file) => file.path === path);
    const next = nextFileIndex(index, files.length, delta);
    if (next >= 0) {
      const nextPath = files[next].path;
      if (onOpenPath) onOpenPath(nextPath);
      else openDiff(nextPath, { preview });
    }
  };

  useDiffKeyboard(
    {
      nextHunk: () => jumpHunk(1),
      prevHunk: () => jumpHunk(-1),
      nextFile: () => moveFile(1),
      prevFile: () => moveFile(-1),
      setMode: changeMode,
      toggleViewed: () => toggleViewed(path),
    },
    active,
  );

  const notice = actions.resendError ?? data.error ?? data.warning;
  return (
    <div className="flex-1 flex flex-col min-h-0 min-w-0 bg-bg" data-diff-tab={path}>
      <div className="min-h-10 shrink-0 border-b border-border flex items-center px-2 gap-2">
        {/* 로드가 실패해도 마지막 스냅샷을 버리지 않는다 — 경고만 얹는다(설계 F-6). */}
        <span className="mr-auto truncate text-xs text-status-failed">{notice}</span>
        {/* 단축키는 보이지 않으면 없는 것과 같다. */}
        <span
          className="h-7 w-7 flex items-center justify-center rounded text-text-muted cursor-help"
          aria-label="단축키 안내"
          title={SHORTCUT_HINT}
        >
          ?
        </span>
        <DiffModeToggle
          mode={mode}
          onChange={changeMode}
          disabled={narrow}
          disabledTitle={NARROW_REASON}
        />
      </div>
      <div ref={scrollRef} className="flex-1 overflow-auto">
        {current ? (
          <DiffBody
            current={current}
            mode={mode}
            fileHunks={fileHunks}
            annotations={annotations}
            partial={partial}
            onCreate={actions.create}
            onUpdate={actions.update}
            onRefresh={data.refresh}
          />
        ) : (
          <VanishedFile loading={data.files === null} onClose={onClose} />
        )}
      </div>
    </div>
  );
}

/** 에이전트가 변경을 되돌리면 파일이 스냅샷에서 빠진다. 탭을 닫지 않는 이유는 사용자가
 *  열어 둔 것이고, 사라졌다는 사실 자체가 알림이기 때문이다(설계 F-1). */
function VanishedFile({ loading, onClose }: { loading: boolean; onClose?: () => void }) {
  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-text-muted">
        변경 사항을 불러오는 중…
      </div>
    );
  }
  return (
    <div className="h-full flex flex-col items-center justify-center gap-3 text-text-muted">
      <span>이 파일은 더 이상 바뀌지 않았습니다.</span>
      {onClose && (
        <button className="rounded border border-border px-3 py-1 text-xs" onClick={onClose}>
          탭 닫기
        </button>
      )}
    </div>
  );
}

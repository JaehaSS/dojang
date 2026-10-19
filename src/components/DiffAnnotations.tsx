import type { DiffHunk, RematchedAnnotation } from "../lib/ipc";
import {
  annotationLine,
  groupAnnotationsByPosition,
  hunkLines,
  orphanedAnnotations,
  positionKey,
  type HunkLine,
} from "../lib/annotations";
import { collapsedLineCount, intraLineMap, type Segment } from "../lib/diff";
import {
  AnnotationComposer,
  AnnotationThread,
  OrphanedAnnotationsNotice,
  useAnnotationComposer,
} from "./DiffAnnotationThread";
import { tokensForLine } from "../lib/diff-highlight";
import { DiffLineText, HunkCheckbox, useDiffTheme, useHunkTokens } from "./DiffPresentation";

/** 라인마다 단어 단위 강조 조각을 붙여 렌더 루프에서 짝짓기 로직이 새지 않게 한다. */
function withIntraLine(lines: HunkLine[]): { line: HunkLine; segments?: Segment[] }[] {
  const map = intraLineMap(lines);
  return lines.map((line, index) => ({ line, segments: map.get(index) }));
}

const lineKindClass: Record<string, string> = {
  add: "bg-addbg text-status-done",
  del: "bg-delbg text-status-failed",
  context: "text-text-secondary",
};

interface Props {
  hunks: DiffHunk[];
  annotations: RematchedAnnotation[];
  onCreate: (input: { hunk_id: string; line: number; side: string; body_md: string }) => Promise<void>;
  onUpdateBody: (id: string, body_md: string) => Promise<void>;
  /** hunk 부분 승인(B-2) 선택 상태 — 생략 시 체크박스를 표시하지 않는다. */
  selectedHunkIds?: Set<string>;
  onToggleHunk?: (hunk: DiffHunk) => void;
  /** 구문 강조 언어(Monaco 언어 id). 생략하면 평문이다. */
  lang?: string;
}

/** unified 모드 diff — 라인 hover 거터 💬, 인라인 마크다운 주석 스레드, 고아 주석 배지,
 *  hunk 부분 승인 체크박스(선택/해제, protected는 비활성 + 사유 툴팁). */
export function AnnotatedUnifiedDiff({
  hunks,
  annotations,
  onCreate,
  onUpdateBody,
  selectedHunkIds,
  onToggleHunk,
  lang = "plaintext",
}: Props) {
  const composer = useAnnotationComposer(onCreate, onUpdateBody);
  const grouped = groupAnnotationsByPosition(annotations);
  const tokens = useHunkTokens(hunks, lang);
  const theme = useDiffTheme();

  return (
    <div className="text-xs font-code leading-relaxed">
      <OrphanedAnnotationsNotice orphaned={orphanedAnnotations(annotations)} />
      {hunks.map((hunk, index) => {
        const previous = hunks[index - 1];
        const gap = previous ? collapsedLineCount(previous.old_range, hunk.old_range) : 0;
        return (
          <div key={hunk.id}>
            {gap > 0 && (
              <div className="my-1 px-3 py-1 text-center bg-raised text-text-muted">
                {gap}개 변경되지 않은 줄
              </div>
            )}
            <div
              data-hunk-id={hunk.id}
              className="px-3 flex items-center gap-2 text-primary-bright bg-empty"
            >
              <HunkCheckbox hunk={hunk} selectedHunkIds={selectedHunkIds} onToggle={onToggleHunk} />
              <span>
                @@ -{hunk.old_range[0]},{hunk.old_range[1]} +{hunk.new_range[0]},{hunk.new_range[1]} @@
              </span>
            </div>
            {withIntraLine(hunkLines(hunk)).map(({ line, segments }, i) => {
              const key = positionKey(hunk.id, annotationLine(line), line.side);
              const thread = grouped.get(key) ?? [];
              const composingHere = composer.activeKey === key;
              return (
                <div key={i}>
                  <div className="group flex items-start px-1 hover:bg-border/40">
                    <button
                      className={`w-4 shrink-0 text-[10px] leading-5 ${
                        thread.length > 0 ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                      }`}
                      title="주석 남기기"
                      onClick={() =>
                        composer.openNew(hunk.id, annotationLine(line), line.side)
                      }
                    >
                      💬
                    </button>
                    <span className="w-10 shrink-0 text-right pr-2 text-text-muted select-none">
                      {line.oldLine ?? ""}
                    </span>
                    <span className="w-10 shrink-0 text-right pr-2 text-text-muted select-none">
                      {line.newLine ?? ""}
                    </span>
                    <span className={`flex-1 whitespace-pre-wrap ${lineKindClass[line.kind]}`}>
                      {line.kind === "add" ? "+" : line.kind === "del" ? "-" : " "}
                      <DiffLineText
                        text={line.text}
                        segments={segments}
                        tokens={tokensForLine(tokens?.get(hunk.id), line, hunk)}
                        theme={theme}
                        kind={line.kind}
                      />
                    </span>
                  </div>
                  <AnnotationThread
                    thread={thread}
                    indent="ml-14"
                    onEdit={(a) =>
                      composer.openEdit(hunk.id, annotationLine(line), line.side, a)
                    }
                  />
                  {composingHere && (
                    <AnnotationComposer
                      indent="ml-14"
                      value={composer.text}
                      error={composer.error}
                      onChange={composer.setText}
                      onSave={composer.save}
                    />
                  )}
                </div>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}

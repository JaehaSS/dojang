// unified diff 파싱/렌더 순수 로직 (DiffTab·ChangesList가 사용, 테스트 대상).

import { hunkLines, type HunkLine } from "./annotations";
import type { DiffHunk } from "./ipc";

export type Side = {
  n: number | null;
  text: string;
  kind: "ctx" | "add" | "del" | "empty" | "hunk" | "gap";
};
export type Row = { left: Side; right: Side };
export interface DiffSummary {
  additions: number;
  deletions: number;
}

/** unified diff 한 줄의 색상 클래스. */
export function lineClass(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff "))
    return "text-text-muted";
  if (line.startsWith("@@")) return "text-primary-bright";
  if (line.startsWith("+")) return "text-status-done bg-addbg";
  if (line.startsWith("-")) return "text-status-failed bg-delbg";
  return "text-text-secondary";
}

export function summarizePatch(patch: string): DiffSummary {
  return patch.split("\n").reduce<DiffSummary>(
    (summary, line) => {
      if (line.startsWith("+") && !line.startsWith("+++")) summary.additions += 1;
      if (line.startsWith("-") && !line.startsWith("---")) summary.deletions += 1;
      return summary;
    },
    { additions: 0, deletions: 0 },
  );
}

export function summarizePatches(patches: string[]): DiffSummary {
  return patches.reduce<DiffSummary>(
    (total, patch) => {
      const summary = summarizePatch(patch);
      return {
        additions: total.additions + summary.additions,
        deletions: total.deletions + summary.deletions,
      };
    },
    { additions: 0, deletions: 0 },
  );
}

export function formatDiffStat(files: { path: string; status: string; patch: string }[]): string {
  if (files.length === 0) return "";
  const summary = summarizePatches(files.map((file) => file.patch));
  const fileWord = files.length === 1 ? "file" : "files";
  return [
    ...files.map((file) => `${file.status}\t${file.path}`),
    `${files.length} ${fileWord} changed, ${summary.additions} insertions(+), ${summary.deletions} deletions(-)`,
  ].join("\n");
}

/** 사이드바가 파일명을 먼저, 디렉터리를 흐리게 뒤에 표시하기 위한 분해.
 *  깊은 경로를 통째로 truncate하면 정작 중요한 파일명 쪽이 잘린다. */
export function splitFilePath(path: string): { name: string; dir: string } {
  const at = path.lastIndexOf("/");
  return at < 0 ? { name: path, dir: "" } : { name: path.slice(at + 1), dir: path.slice(0, at) };
}

export function describePatch(patch: string): string | null {
  if (patch.includes("Praxis diff omitted: file exceeds 1 MiB")) {
    return "파일이 1 MiB를 초과하여 내용 diff를 표시할 수 없습니다.";
  }
  if (/Binary files |GIT binary patch/.test(patch)) {
    return "바이너리 파일 — 내용 diff를 표시할 수 없습니다.";
  }
  const from = patch.match(/^rename from (.+)$/m)?.[1];
  const to = patch.match(/^rename to (.+)$/m)?.[1];
  if (from && to) return `파일 이름이 변경되었습니다: ${from} → ${to}`;
  const oldMode = patch.match(/^old mode (.+)$/m)?.[1];
  const newMode = patch.match(/^new mode (.+)$/m)?.[1];
  if (oldMode && newMode) return `파일 모드가 변경되었습니다: ${oldMode} → ${newMode}`;
  return patch.trim() ? "텍스트 변경 구간이 없는 메타데이터 diff입니다." : "표시할 텍스트 diff가 없습니다.";
}

export function collapsedLineCount(previous: [number, number], next: [number, number]): number {
  return Math.max(0, next[0] - (previous[0] + previous[1]));
}

export interface Segment {
  text: string;
  changed: boolean;
}

const WORD = /[A-Za-z0-9_$가-힣]/;

/** index가 서로게이트 페어 한가운데인가 — 여기서 자르면 이모지가 반쪽 글자로 깨진다. */
function splitsSurrogatePair(text: string, index: number): boolean {
  if (index <= 0 || index >= text.length) return false;
  const before = text.charCodeAt(index - 1);
  const at = text.charCodeAt(index);
  return before >= 0xd800 && before <= 0xdbff && at >= 0xdc00 && at <= 0xdfff;
}

function segmentsOf(text: string, from: number, to: number): Segment[] {
  const out: Segment[] = [];
  if (from > 0) out.push({ text: text.slice(0, from), changed: false });
  if (to > from) out.push({ text: text.slice(from, to), changed: true });
  if (to < text.length) out.push({ text: text.slice(to), changed: false });
  return out;
}

/** 짝지어진 del/add 줄에서 실제로 바뀐 문자 구간만 골라낸다. 공통 접두·접미를 벗기는
 *  단순한 방식이며 LCS를 쓰지 않는다 — 한 줄 안의 국소 수정을 짚는 데는 이걸로 충분하다.
 *
 *  줄 대부분이 바뀌었으면 null을 돌려준다. 그때는 줄 전체 색만으로 충분하고, 조각 강조는
 *  오히려 시선을 흩뜨린다. 이 판정은 단어 경계 확장 *이전* 구간을 기준으로 한다 —
 *  확장은 가독성을 위해 범위를 넓히는 것이라 판정 기준으로 삼으면 스스로를 무효화한다. */
export function intraLineSegments(
  before: string,
  after: string,
): { before: Segment[]; after: Segment[] } | null {
  if (before === after) return null;

  const max = Math.min(before.length, after.length);
  let start = 0;
  while (start < max && before[start] === after[start]) start += 1;
  let end = 0;
  while (end < max - start && before[before.length - 1 - end] === after[after.length - 1 - end])
    end += 1;

  const changed = before.length - end - start + (after.length - end - start);
  if (changed > (before.length + after.length) * 0.6) return null;

  // 단어 중간에서 잘렸으면 단어 시작까지 되돌린다 — "useState"→"useStates"에서 "s" 한 글자만
  // 튀는 것을 막고 바뀐 식별자 전체를 보여 준다.
  while (
    start > 0 &&
    WORD.test(before[start - 1]) &&
    (WORD.test(before[start] ?? "") || WORD.test(after[start] ?? ""))
  )
    start -= 1;

  // 경계가 서로게이트 페어를 쪼개면 강조를 포기한다. 코드포인트 단위로 되돌리는 것보다
  // 줄 틴트만 남기는 편이 낫다 — 반쪽 서로게이트는 화면에 깨진 글자로 나온다.
  const cuts: [string, number][] = [
    [before, start],
    [before, before.length - end],
    [after, start],
    [after, after.length - end],
  ];
  if (cuts.some(([text, at]) => splitsSurrogatePair(text, at))) return null;

  return {
    before: segmentsOf(before, start, before.length - end),
    after: segmentsOf(after, start, after.length - end),
  };
}

/** 통합 모드용 — 인접한 del 블록과 add 블록을 k번째끼리 짝지어 단어 단위 강조를 계산한다.
 *  반환은 `hunkLines` 배열 인덱스 → Segment[]. 짝이 없거나 통째로 바뀐 줄은 담기지 않는다
 *  (분할 모드는 행이 이미 좌우로 짝지어져 있어 intraLineSegments를 직접 부른다). */
export function intraLineMap(lines: HunkLine[]): Map<number, Segment[]> {
  const map = new Map<number, Segment[]>();
  let i = 0;
  while (i < lines.length) {
    if (lines[i].kind !== "del") {
      i += 1;
      continue;
    }
    const delStart = i;
    while (i < lines.length && lines[i].kind === "del") i += 1;
    const addStart = i;
    while (i < lines.length && lines[i].kind === "add") i += 1;
    for (let k = 0; k < Math.min(addStart - delStart, i - addStart); k++) {
      const pair = intraLineSegments(lines[delStart + k].text, lines[addStart + k].text);
      if (!pair) continue;
      map.set(delStart + k, pair.before);
      map.set(addStart + k, pair.after);
    }
  }
  return map;
}

/** 분할 모드 한 칸 — 주석을 걸 좌표(hunk id + side)를 함께 나른다. */
export interface SplitCell {
  hunkId: string;
  line: HunkLine;
}
export type SplitRow =
  | { kind: "gap"; text: string }
  | { kind: "hunk"; hunk: DiffHunk; text: string }
  | { kind: "pair"; left: SplitCell | null; right: SplitCell | null };

/** DiffHunk[]를 좌우(old|new) 행으로 변환. toSplitRows(patch)와 달리 hunk id와 라인 side를
 *  유지하므로 분할 모드에서도 주석·부분 승인을 걸 수 있다. context 행은 좌우가 같은 객체다
 *  — 호출부는 `left === right`로 판별해 스레드를 한 번만 렌더한다. */
export function toSplitRowsFromHunks(hunks: DiffHunk[]): SplitRow[] {
  const rows: SplitRow[] = [];
  hunks.forEach((hunk, index) => {
    const previous = hunks[index - 1];
    const gap = previous ? collapsedLineCount(previous.old_range, hunk.old_range) : 0;
    if (gap > 0) rows.push({ kind: "gap", text: `${gap}개 변경되지 않은 줄` });
    rows.push({
      kind: "hunk",
      hunk,
      text: `@@ -${hunk.old_range[0]},${hunk.old_range[1]} +${hunk.new_range[0]},${hunk.new_range[1]} @@`,
    });

    const lines = hunkLines(hunk);
    let i = 0;
    while (i < lines.length) {
      if (lines[i].kind === "context") {
        const cell: SplitCell = { hunkId: hunk.id, line: lines[i] };
        rows.push({ kind: "pair", left: cell, right: cell });
        i += 1;
        continue;
      }
      const dels: HunkLine[] = [];
      const adds: HunkLine[] = [];
      while (i < lines.length && lines[i].kind === "del") dels.push(lines[i++]);
      while (i < lines.length && lines[i].kind === "add") adds.push(lines[i++]);
      for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
        rows.push({
          kind: "pair",
          left: k < dels.length ? { hunkId: hunk.id, line: dels[k] } : null,
          right: k < adds.length ? { hunkId: hunk.id, line: adds[k] } : null,
        });
      }
    }
  });
  return rows;
}

/** unified patch를 좌우(old|new) 행으로 변환. */
export function toSplitRows(patch: string): Row[] {
  const rows: Row[] = [];
  const lines = patch.split("\n");
  let lo = 0;
  let ro = 0;
  let i = 0;
  let previousOldRange: [number, number] | null = null;
  while (i < lines.length) {
    const line = lines[i];
    if (line.startsWith("@@")) {
      const m = line.match(/@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
      if (m) {
        const nextOldRange: [number, number] = [parseInt(m[1], 10), parseInt(m[2] ?? "1", 10)];
        const gap = previousOldRange ? collapsedLineCount(previousOldRange, nextOldRange) : 0;
        if (gap > 0) {
          const text = `${gap}개 변경되지 않은 줄`;
          rows.push({
            left: { n: null, text, kind: "gap" },
            right: { n: null, text, kind: "gap" },
          });
        }
        lo = parseInt(m[1], 10);
        ro = parseInt(m[3], 10);
        previousOldRange = nextOldRange;
      }
      rows.push({ left: { n: null, text: line, kind: "hunk" }, right: { n: null, text: "", kind: "hunk" } });
      i++;
      continue;
    }
    if (/^(diff |index |--- |\+\+\+ |new file|deleted file|similarity|rename )/.test(line)) {
      i++;
      continue;
    }
    if (line.startsWith(" ")) {
      rows.push({
        left: { n: lo, text: line.slice(1), kind: "ctx" },
        right: { n: ro, text: line.slice(1), kind: "ctx" },
      });
      lo++;
      ro++;
      i++;
      continue;
    }
    if (line.startsWith("-") || line.startsWith("+")) {
      const dels: string[] = [];
      const adds: string[] = [];
      while (i < lines.length && lines[i].startsWith("-")) {
        dels.push(lines[i].slice(1));
        i++;
      }
      while (i < lines.length && lines[i].startsWith("+")) {
        adds.push(lines[i].slice(1));
        i++;
      }
      const max = Math.max(dels.length, adds.length);
      for (let k = 0; k < max; k++) {
        const left: Side =
          k < dels.length ? { n: lo++, text: dels[k], kind: "del" } : { n: null, text: "", kind: "empty" };
        const right: Side =
          k < adds.length ? { n: ro++, text: adds[k], kind: "add" } : { n: null, text: "", kind: "empty" };
        rows.push({ left, right });
      }
      continue;
    }
    i++;
  }
  return rows;
}

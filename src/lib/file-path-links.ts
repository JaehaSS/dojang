/** 에이전트 답변에 평문으로 적힌 파일 경로("플랜 저장 완료: docs/plans/0014.…md")를
 *  클릭 가능한 링크로 바꾼다. 링크가 된 뒤는 기존 경로를 그대로 탄다 —
 *  Markdown의 onOpenLink → resolveAgentLink(worktree 안으로 제한) → 에디터 탭.
 *
 *  링크로 볼 조건은 **슬래시 하나 + 확장자**다. 둘 다 요구하는 이유는 오탐 비용이 크기 때문 —
 *  "and/or"·"Q4/2026"은 확장자가 없어 걸리지 않고, "Next.js"·"socket.io"는 슬래시가 없어 걸리지
 *  않는다. 대신 루트 직속 파일(`package.json`, `CLAUDE.md`)은 링크되지 않는다. 이건 의도된
 *  트레이드오프다 — 문장 속 제품명을 죽은 링크로 만드는 쪽이 더 나쁘다.
 *
 *  디렉터리·파일 이름은 **모든 문자 체계의 글자와 숫자**를 받는다(`\p{L}\p{N}`). ASCII만 받던
 *  동안 `문서/학습자료/평가-체계.md` 같은 한글 경로는 세그먼트 하나만 한글이어도 통째로 빠져,
 *  볼트 문서 링크가 대화 뷰에서 평문으로만 남았다. 확장자만은 ASCII로 남긴다 — 한글 조사가
 *  확장자 뒤에 붙는 `docs/STATE.md에서`를 `.md에서`로 삼키지 않기 위한 경계다.
 */

const NAME_CHARS = "\\p{L}\\p{N}._~@+-";
const SEGMENT = `[${NAME_CHARS}]+`;
const FILE_NAME = `[${NAME_CHARS}]*\\.[A-Za-z][A-Za-z0-9]{0,7}`;
/** `src/App.tsx:1187`, `:1187:5` — 답변에서 흔한 착지 지점 표기. */
const LINE_SUFFIX = "(?::\\d{1,7}(?::\\d{1,7})?)?";
const PATH_SOURCE = `/?(?:${SEGMENT}/)+${FILE_NAME}${LINE_SUFFIX}`;

/** 매치 왼쪽에 이 문자가 붙어 있으면 더 큰 토큰의 조각이다 —
 *  URL(`https://host/a.md`)이나 SCP 주소(`git@host:org/repo.git`)의 경로 부분. */
const JOINED_LEFT = /[\p{L}\p{N}._~@+/:\\-]/u;

export interface FilePathSpan {
  /** 원문에서의 시작 오프셋 */
  start: number;
  /** 원문에서의 끝 오프셋(제외) */
  end: number;
  /** 매치된 경로 원문 — 링크 텍스트와 href에 그대로 쓴다(라인 접미 포함) */
  text: string;
}

/** 한 텍스트 조각에서 파일 경로로 볼 구간을 왼쪽부터 찾는다. */
export function findFilePathSpans(value: string): FilePathSpan[] {
  const re = new RegExp(PATH_SOURCE, "gu");
  const spans: FilePathSpan[] = [];
  for (let match = re.exec(value); match; match = re.exec(value)) {
    const start = match.index;
    // 앞이 붙어 있으면 버린다. lastIndex는 이미 매치 끝이므로 그 뒤부터 다시 찾는다.
    if (start > 0 && JOINED_LEFT.test(value[start - 1])) continue;
    spans.push({ start, end: start + match[0].length, text: match[0] });
  }
  return spans;
}

const WHOLE_PATH = new RegExp(`^${PATH_SOURCE}$`, "u");

/** 백틱이 경계를 대신 그어 주므로 인라인 코드 안에서는 공백을 품은 이름도 받는다
 *  (`~/Documents/째하 지식창고/…`). 다만 `~/`·`/`로 시작하는 절대경로만이다 — 상대경로까지 받으면
 *  `git add src/a.ts`가 통째로 `git add src` 디렉터리의 경로가 된다. 공백은 디렉터리 이름 안의
 *  단어 사이 한 칸만이다 — 파일 이름까지 받으면 `/usr/bin/env python3.11` 같은 명령이 경로가 된다. */
const SPACED_SEGMENT = `[${NAME_CHARS}]+(?: [${NAME_CHARS}]+)*`;
const WHOLE_SPACED_PATH = new RegExp(
  `^(?:~/|/)(?:${SPACED_SEGMENT}/)*${FILE_NAME}${LINE_SUFFIX}$`,
  "u",
);

/** `docs/plans/`처럼 `/`로 끝나는 인라인 코드 — 뒤따르는 맨 파일 이름의 기준 디렉터리가 된다. */
const WHOLE_DIRECTORY = new RegExp(
  `^(?:/?(?:${SEGMENT}/)+|(?:~/|/)(?:${SPACED_SEGMENT}/)+)$`,
  "u",
);

/** `README.md`, `notes.md:12` — 슬래시 없는 파일 이름. 기준 디렉터리가 있을 때만 링크가 된다. */
const WHOLE_BARE_NAME = new RegExp(`^${FILE_NAME}${LINE_SUFFIX}$`, "u");

/** 인라인 코드는 **내용 전체가** 경로일 때만 링크로 만든다.
 *  `npm run docs:project`처럼 경로를 품은 명령을 반쪼가리 링크로 쪼개지 않기 위한 규칙. */
export function isFilePathOnly(value: string): boolean {
  const trimmed = value.trim();
  return WHOLE_PATH.test(trimmed) || WHOLE_SPACED_PATH.test(trimmed);
}

/** 인라인 코드 전체가 디렉터리 경로면 그 경로를(끝 `/` 포함), 아니면 null. */
export function directoryOnly(value: string): string | null {
  const trimmed = value.trim();
  return WHOLE_DIRECTORY.test(trimmed) ? trimmed : null;
}

/** mdast에서 실제로 쓰는 부분만. @types/mdast에 얹지 않아 remark 버전과 독립. */
interface MdNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
}

/** 에이전트가 흔히 쓰는 "머리말에 디렉터리 하나, 목록에 파일 이름들" 형식을 위한 문맥.
 *
 *  ```
 *  **만든 문서** (`~/Documents/볼트/웹어셈블리/`)
 *  - `README.md`: 목차
 *  ```
 *
 *  맨 파일 이름은 이 디렉터리 기준으로만 링크된다. 문맥은 최상위 블록 하나와, 바로 뒤에 붙은
 *  목록까지만 산다 — 한참 아래 문단의 `package.json`이 엉뚱한 디렉터리로 이어지지 않게. */
interface LinkContext {
  base: string | null;
}

/** 텍스트/인라인 코드 노드의 파일 경로를 link 노드로 감싸는 remark 플러그인.
 *  remarkGfm 뒤에 둘 것 — URL이 먼저 link가 되어야 그 안을 건드리지 않는다. */
export function remarkFilePathLinks() {
  return (tree: unknown) => {
    const root = tree as MdNode;
    const context: LinkContext = { base: null };
    for (const block of root.children ?? []) {
      if (block.type !== "list") context.base = null;
      linkify(block, context);
    }
  };
}

function linkify(node: MdNode, context: LinkContext): void {
  const children = node.children;
  if (!children) return;
  // 링크 안에서 링크를 만들 수는 없다. 이미 사람이(또는 gfm이) 링크로 만든 것은 그대로 둔다.
  if (node.type === "link" || node.type === "linkReference") return;

  const next: MdNode[] = [];
  let changed = false;

  for (const child of children) {
    if (child.type === "text" && typeof child.value === "string") {
      const split = splitText(child.value);
      if (split) {
        next.push(...split);
        changed = true;
        continue;
      }
    } else if (child.type === "inlineCode" && typeof child.value === "string") {
      const url = inlineCodeUrl(child.value, context);
      if (url) {
        next.push({ type: "link", url, children: [child] });
        changed = true;
        continue;
      }
    } else {
      linkify(child, context);
    }
    next.push(child);
  }

  if (changed) node.children = next;
}

/** 인라인 코드가 가리키는 링크 대상. 디렉터리면 문맥만 바꾸고 링크는 만들지 않는다. */
function inlineCodeUrl(value: string, context: LinkContext): string | null {
  if (isFilePathOnly(value)) return value.trim();
  const directory = directoryOnly(value);
  if (directory) {
    context.base = directory;
    return null;
  }
  const name = value.trim();
  return context.base && WHOLE_BARE_NAME.test(name) ? context.base + name : null;
}

/** 경로를 찾았을 때만 노드 배열을 돌려준다(못 찾으면 null → 원본 노드 유지). */
function splitText(value: string): MdNode[] | null {
  const spans = findFilePathSpans(value);
  if (!spans.length) return null;

  const nodes: MdNode[] = [];
  let cursor = 0;
  for (const span of spans) {
    if (span.start > cursor) {
      nodes.push({ type: "text", value: value.slice(cursor, span.start) });
    }
    nodes.push({
      type: "link",
      url: span.text,
      children: [{ type: "text", value: span.text }],
    });
    cursor = span.end;
  }
  if (cursor < value.length) nodes.push({ type: "text", value: value.slice(cursor) });
  return nodes;
}

import { memo, type ReactNode, useMemo, useSyncExternalStore } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { remarkFilePathLinks } from "../../lib/file-path-links";
import { getActiveTheme, subscribeTheme } from "../../lib/themes";
import { looksLikeHtml } from "../../lib/looks-like-html";
import { rehypeExpressions } from "../../lib/vocab";
import { CodeCopyButton } from "./CodeCopyButton";
import { HtmlDoc } from "./HtmlDoc";
import { extractText, type CodeChild } from "./markdown-code";
import { Mermaid } from "./Mermaid";

function SessionMermaid({ chart }: { chart: string }) {
  const theme = useSyncExternalStore(subscribeTheme, getActiveTheme, getActiveTheme);

  return <Mermaid chart={chart} dark={theme.kind === "dark"} />;
}

/** 펜스 코드블록 — Mermaid는 도표로, 나머지는 언어 라벨·복사 버튼이 있는 코드로 표시한다. */
function CodeBlock({ children }: { children?: ReactNode }) {
  const child = (Array.isArray(children) ? children[0] : children) as CodeChild | undefined;
  const className = child?.props?.className ?? "";
  const lang = /language-(\w+)/.exec(className)?.[1];
  const source = extractText(child?.props?.children);

  if (lang?.toLowerCase() === "mermaid") return <SessionMermaid chart={source} />;

  return (
    <div className="relative my-1.5 overflow-hidden rounded-md border border-border bg-bg">
      <CodeCopyButton text={source} />
      {lang && (
        <div className="px-2.5 pt-1.5 pr-9 text-[10px] uppercase tracking-wide text-text-muted">
          {lang}
        </div>
      )}
      <div className="overflow-x-auto pb-3">
        <pre className="m-0 text-xs font-code">
          <code className="block px-2.5 py-2 text-text whitespace-pre">
            {child?.props?.children}
          </code>
        </pre>
      </div>
    </div>
  );
}

/** 풀 CommonMark + GFM(표/취소선/체크리스트) + 채팅식 줄바꿈(remark-breaks). 로컬 번들.
 *  인라인 코드는 컨테이너 셀렉터로 스타일 — pre 내부 코드와 충돌하지 않게(components.code 미사용).
 *  memo: 스트리밍/히스토리에서 이전 블록의 text가 그대로면 재파싱하지 않는다(긴 대화 리렌더 방지, perf). */
interface Props {
  text: string;
  onOpenLink?: (link: string) => void;
  /** 링크 우클릭 — 좌표와 링크 원문을 위로 올린다. 메뉴를 무엇으로 채울지는 App이 정한다. */
  onLinkMenu?: (link: string, at: { x: number; y: number }) => void;
  /** 이 텍스트가 더 자라지 않는가. 스트리밍 중이면 거짓을 준다 — 아래 HTML 분기 주석 참조. */
  stable?: boolean;
  /**
   * 공부할 표현 — 글자 노드에서 찾아 `renderHighlight`로 감싼다(`src/lib/vocab.ts`). 코드·링크 안은 긋지 않는다.
   * 배열 정체성이 바뀌면 다시 파싱하므로 부모가 고정해서 준다.
   */
  highlights?: string[];
  renderHighlight?: (index: number, children: ReactNode) => ReactNode;
}

const agentUrlTransform = (value: string) =>
  /^file:/i.test(value) ? value : defaultUrlTransform(value);

/** 평문 경로 링크화는 열어줄 손잡이(onOpenLink)가 있을 때만 켠다 —
 *  없으면 클릭해도 아무 일이 없거나 상대 URL을 브라우저로 던지는 죽은 링크가 된다. */
const PLAIN_PLUGINS = [remarkGfm, remarkBreaks];
const AGENT_PLUGINS = [remarkGfm, remarkBreaks, remarkFilePathLinks];

export const Markdown = memo(function Markdown({
  text,
  onOpenLink,
  onLinkMenu,
  stable = true,
  highlights,
  renderHighlight,
}: Props) {
  const rehypePlugins = useMemo(
    () => (highlights && highlights.length > 0 && renderHighlight ? [rehypeExpressions(highlights)] : undefined),
    [highlights, renderHighlight],
  );

  // 메시지 전체가 HTML이면 마크다운 파서는 태그를 글자로 이스케이프해 버린다 — 샌드박스로 넘긴다.
  //
  // 자라는 중인 버퍼로는 판정하지 않는다(stable=false). "표를 그리고 문장으로 마무리"하는 답변은
  // 버퍼가 잠깐 `</table>`로 끝나는 순간이 있어, 그때 360px iframe이 됐다가 다음 토큰에 텍스트로
  // 돌아온다 — 대화가 그 높이만큼 튀고 스크롤 앵커가 흔들린다.
  //
  // 높이가 고정인 이유: 빈 sandbox는 불투명 출처라 부모가 내용 높이를 잴 수 없다(HtmlDoc 주석).
  if (stable && looksLikeHtml(text)) {
    return (
      <div className="my-1.5 rounded-md border border-border overflow-hidden h-[360px]">
        <HtmlDoc html={text} title="HTML 미리보기" className="w-full h-full border-0 bg-white" />
      </div>
    );
  }

  return (
    <div
      className="leading-relaxed break-words
        [&_:not(pre)>code]:px-1 [&_:not(pre)>code]:py-0.5 [&_:not(pre)>code]:rounded
        [&_:not(pre)>code]:bg-raised [&_:not(pre)>code]:text-primary-bright
        [&_:not(pre)>code]:font-code [&_:not(pre)>code]:text-[0.88em]"
    >
      <ReactMarkdown
        remarkPlugins={onOpenLink ? AGENT_PLUGINS : PLAIN_PLUGINS}
        rehypePlugins={rehypePlugins}
        urlTransform={onOpenLink ? agentUrlTransform : undefined}
        components={{
          pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
          p: ({ children }) => <p className="my-1">{children}</p>,
          h1: ({ children }) => <h1 className="mt-2 mb-1 text-base font-semibold">{children}</h1>,
          h2: ({ children }) => <h2 className="mt-2 mb-1 text-[15px] font-semibold">{children}</h2>,
          h3: ({ children }) => <h3 className="mt-1.5 mb-1 text-sm font-semibold">{children}</h3>,
          h4: ({ children }) => <h4 className="mt-1.5 mb-0.5 text-sm font-medium">{children}</h4>,
          ul: ({ children }) => <ul className="my-1 pl-5 list-disc space-y-0.5">{children}</ul>,
          ol: ({ children }) => <ol className="my-1 pl-5 list-decimal space-y-0.5">{children}</ol>,
          li: ({ children }) => <li className="[&>p]:my-0">{children}</li>,
          a: ({ href, children }) => (
            <a
              href={href}
              target="_blank"
              rel="noreferrer"
              className="text-primary-bright underline underline-offset-2"
              onClick={
                onOpenLink && href
                  ? (event) => {
                      event.preventDefault();
                      onOpenLink(href);
                    }
                  : undefined
              }
              // 붙이지 않으면 WebKit 기본 메뉴가 뜬다 — 열 손잡이가 없을 때는 그것이 맞다.
              onContextMenu={
                onLinkMenu && href
                  ? (event) => {
                      event.preventDefault();
                      onLinkMenu(href, { x: event.clientX, y: event.clientY });
                    }
                  : undefined
              }
            >
              {children}
            </a>
          ),
          blockquote: ({ children }) => (
            <blockquote className="my-1.5 border-l-2 border-border pl-3 text-text-secondary">
              {children}
            </blockquote>
          ),
          hr: () => <hr className="my-2 border-border" />,
          table: ({ children }) => (
            <div className="my-1.5 overflow-auto">
              <table className="text-xs border-collapse">{children}</table>
            </div>
          ),
          th: ({ children }) => (
            <th className="border border-border bg-raised px-2 py-1 text-left font-medium">
              {children}
            </th>
          ),
          td: ({ children }) => <td className="border border-border px-2 py-1 align-top">{children}</td>,
          mark: ({ node, children }) => {
            const index = Number(node?.properties?.dataExpression);
            return renderHighlight && Number.isInteger(index) ? renderHighlight(index, children) : <mark>{children}</mark>;
          },
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});

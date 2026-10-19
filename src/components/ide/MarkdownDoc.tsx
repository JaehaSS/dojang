import { memo, type ReactNode } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { looksLikeHtml } from "../../lib/looks-like-html";
import { HtmlDoc } from "./HtmlDoc";
import { Mermaid } from "./Mermaid";
import { extractText, type CodeChild } from "./markdown-code";
import { MD_LINE_ATTR, MD_LINE_END_ATTR } from "../../lib/md-selection";
import { MarkdownImage, type DocumentImageReader } from "./MarkdownImage";

/** hast 노드가 들고 있는 원본 위치를 DOM 속성으로 남긴다.
 *
 * 프리뷰에서 고른 텍스트가 파일 어디인지 되찾는 유일한 통로다(`lib/md-selection`) — 렌더된
 * 글자는 원본과 다르므로 텍스트로는 되찾을 수 없다. 위치가 없는 노드도 있으므로(플러그인이
 * 만들어 낸 것) undefined면 속성 자체가 붙지 않는 React의 동작에 맡긴다. */
type MdNodeProps = { node?: { position?: { start: { line: number }; end: { line: number } } } };

const lineProps = (props: MdNodeProps) => ({
  [MD_LINE_ATTR]: props.node?.position?.start.line,
  [MD_LINE_END_ATTR]: props.node?.position?.end.line,
});

/** 펜스 코드블록 — 문서 스케일. language-mermaid는 다이어그램으로, 그 외는 코드 박스로 렌더. */
function CodeBlock({
  children,
  dark,
  allowDiagrams,
  ...rest
}: { children?: ReactNode; dark: boolean; allowDiagrams: boolean } & Record<string, unknown>) {
  const child = (Array.isArray(children) ? children[0] : children) as CodeChild | undefined;
  const className = child?.props?.className ?? "";
  const lang = /language-(\w+)/.exec(className)?.[1];

  // Mermaid 이미지 노드는 strict 모드에서도 렌더 도중 네트워크 요청을 만든다.
  // 외부 이미지가 금지된 문서는 다이어그램 엔진을 실행하지 않고 원문을 보인다.
  if (lang === "mermaid" && allowDiagrams) {
    return (
      <div {...rest}>
        <Mermaid chart={extractText(child?.props?.children)} dark={dark} />
      </div>
    );
  }

  return (
    <div className="my-2 overflow-hidden rounded-md border border-border bg-bg" {...rest}>
      {lang && (
        <div className="px-3 pt-2 text-[11px] uppercase tracking-wide text-text-muted">{lang}</div>
      )}
      <div className="overflow-x-auto pb-3">
        <pre className="m-0 text-sm font-code">
          <code className="block px-3 py-2.5 text-text whitespace-pre">
            {child?.props?.children}
          </code>
        </pre>
      </div>
    </div>
  );
}

/** 파일 뷰어용 문서 스케일 마크다운 렌더러. 풀 CommonMark + GFM(표/취소선/체크리스트).
 *  채팅용 Markdown.tsx와 달리 remark-breaks 미사용(표준 CommonMark 줄바꿈 규칙 — 문서 렌더에 적합).
 *  memo: text 동일 시 재파싱 방지. */
interface Props {
  text: string;
  dark: boolean;
  loadImages?: boolean;
  documentPath?: string;
  rootPath?: string | null;
  readFile?: DocumentImageReader;
  onLinkMenu?: (link: string, at: { x: number; y: number }) => void;
  /** 링크를 왼쪽 클릭했다. 없으면 앵커의 기본 동작을 그대로 둔다. */
  onOpenLink?: (link: string) => void;
}

export const MarkdownDoc = memo(function MarkdownDoc({ text, dark, loadImages = true, documentPath, rootPath, readFile, onLinkMenu, onOpenLink }: Props) {
  // 본문 전체가 HTML이면 마크다운 파서가 태그를 글자로 이스케이프한다 — 샌드박스로 넘긴다.
  // (EditorPane은 그 위에서 이미 가로채므로 여기 오는 건 위키 패널 같은 경로뿐이다.)
  //
  // loadImages=false면 넘기지 않는다. `sandbox=""`가 막는 것은 스크립트·폼·동일 출처이지
  // 하위 리소스 로드가 아니다 — `<img src="https://…">`·`<link rel=stylesheet>`·CSS `url()`은
  // 그대로 나간다. 외부 이미지를 금지한 문서에서 태그가 글자로 보이는 것은, mermaid를 원문으로
  // 보여주는 것과 같은 판단이다(위 CodeBlock).
  //
  // 높이 고정: 빈 sandbox는 불투명 출처라 부모가 내용 높이를 잴 수 없다(HtmlDoc 주석).
  if (loadImages && looksLikeHtml(text)) {
    return (
      <div className="my-2 rounded-md border border-border overflow-hidden h-[70vh]">
        <HtmlDoc html={text} title="HTML 미리보기" className="w-full h-full border-0 bg-white" />
      </div>
    );
  }

  return (
    <div
      className="leading-relaxed break-words text-sm font-ui
        [&_:not(pre)>code]:px-1.5 [&_:not(pre)>code]:py-0.5 [&_:not(pre)>code]:rounded
        [&_:not(pre)>code]:bg-raised [&_:not(pre)>code]:text-primary-bright
        [&_:not(pre)>code]:font-code [&_:not(pre)>code]:text-[0.9em]"
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={(value, key, node) =>
          key === "src" && node.tagName === "img" && /^(?:file:|data:image\/)/i.test(value)
            ? value
            : defaultUrlTransform(value)
        }
        components={{
          pre: (props) => (
            <CodeBlock dark={dark} allowDiagrams={loadImages} {...lineProps(props)}>
              {props.children}
            </CodeBlock>
          ),
          p: (props) => (
            <p className="my-2" {...lineProps(props)}>
              {props.children}
            </p>
          ),
          h1: (props) => (
            <h1 className="mt-4 mb-2 text-2xl font-semibold" {...lineProps(props)}>
              {props.children}
            </h1>
          ),
          h2: (props) => (
            <h2 className="mt-3.5 mb-1.5 text-xl font-semibold" {...lineProps(props)}>
              {props.children}
            </h2>
          ),
          h3: (props) => (
            <h3 className="mt-3 mb-1.5 text-lg font-semibold" {...lineProps(props)}>
              {props.children}
            </h3>
          ),
          h4: (props) => (
            <h4 className="mt-2.5 mb-1 text-base font-medium" {...lineProps(props)}>
              {props.children}
            </h4>
          ),
          ul: (props) => (
            <ul className="my-2 pl-6 list-disc space-y-1" {...lineProps(props)}>
              {props.children}
            </ul>
          ),
          ol: (props) => (
            <ol className="my-2 pl-6 list-decimal space-y-1" {...lineProps(props)}>
              {props.children}
            </ol>
          ),
          li: (props) => (
            <li className="[&>p]:my-0" {...lineProps(props)}>
              {props.children}
            </li>
          ),
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
          img: (props) => loadImages ? (
            <MarkdownImage
              src={typeof props.src === "string" ? props.src : undefined}
              alt={props.alt}
              title={props.title}
              documentPath={documentPath}
              rootPath={rootPath}
              readFile={readFile}
              {...lineProps(props)}
            />
          ) : <span className="text-text-muted">[이미지: {props.alt || "외부 이미지"}]</span>,
          blockquote: (props) => (
            <blockquote
              className="my-2 border-l-2 border-border-strong pl-4 text-text-secondary"
              {...lineProps(props)}
            >
              {props.children}
            </blockquote>
          ),
          hr: () => <hr className="my-4 border-border" />,
          table: (props) => (
            <div className="my-2 overflow-auto rounded-md border border-border">
              <table className="w-full text-sm border-collapse" {...lineProps(props)}>
                {props.children}
              </table>
            </div>
          ),
          th: (props) => (
            <th
              className="border border-border bg-raised px-3 py-1.5 text-left font-medium"
              {...lineProps(props)}
            >
              {props.children}
            </th>
          ),
          td: (props) => (
            <td className="border border-border px-3 py-1.5 align-top" {...lineProps(props)}>
              {props.children}
            </td>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});

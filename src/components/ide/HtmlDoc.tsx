import { memo, useEffect, useRef, useState } from "react";
import { buildHtmlPreview, type HtmlAssetReader, type HtmlPreviewResult } from "../../lib/html-preview-assets";

/** 파일 뷰어용 HTML 렌더러 — 정적 리소스를 준비해 샌드박스 iframe에 표시한다.
 *
 *  `sandbox=""`(빈 값)이 이 기능의 기본 안전 경계다. 워크트리의 HTML은 에이전트가 만들었든
 *  받아왔든 신뢰할 수 없는 입력이므로 `allow-scripts`·`allow-same-origin`을 **절대 넣지 않는다** —
 *  빈 값이면 스크립트 실행·폼 제출·상위 프레임 내비게이션·팝업이 모두 막히고 불투명 출처가 되어
 *  앱의 DOM·스토리지에 닿을 수 없다.
 *
 *  파일 에디터는 기존 파일 읽기 경계로 CSS·이미지를 인라인한다. 경로/reader가 없는 채팅·위키는
 *  원문을 그대로 표시한다. srcdoc의 기본 URL은 앱 URL이라 프로젝트 자원을 직접 가리키지 않는다.
 *
 *  bg-white: 문서가 자기 배경을 지정하지 않으면 앱의 어두운 배경이 비쳐 검은 글자가 안 보인다.
 *  브라우저와 같이 흰 페이지 위에 얹는다.
 *
 *  className: 호출자가 크기를 정한다. 빈 sandbox의 불투명 출처라 부모가 `contentDocument`를 읽을
 *  수 없고, 따라서 **내용에 맞춘 자동 높이 조절이 불가능하다** — 전면 표시가 아닌 자리(채팅 버블
 *  등)는 고정 높이를 주고 안쪽 스크롤에 맡겨야 한다.
 *
 *  memo: html 동일 시 iframe 재로드 방지. */
export const HtmlDoc = memo(function HtmlDoc({
  html,
  title,
  retainKeyboardFocus = false,
  className = "w-full h-full border-0 bg-white",
  sourcePath,
  rootPath,
  readAsset,
  revision,
}: {
  html: string; title: string; className?: string; retainKeyboardFocus?: boolean;
  sourcePath?: string; rootPath?: string | null; readAsset?: HtmlAssetReader; revision?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement>(null);
  const [loaded, setLoaded] = useState<{
    html: string; sourcePath: string; rootPath?: string | null; readAsset: HtmlAssetReader;
    revision?: string; result: HtmlPreviewResult;
  } | null>(null);
  const prepare = !!sourcePath && !!readAsset;
  const result = loaded?.html === html && loaded.sourcePath === sourcePath && loaded.rootPath === rootPath
    && loaded.readAsset === readAsset && loaded.revision === revision ? loaded.result : null;
  useEffect(() => {
    if (!sourcePath || !readAsset) return;
    const controller = new AbortController();
    void buildHtmlPreview(html, sourcePath, readAsset, rootPath, controller.signal).then((next) => {
      if (!controller.signal.aborted) setLoaded({ html, sourcePath, rootPath, readAsset, revision, result: next });
    }).catch(() => {
      if (!controller.signal.aborted) setLoaded({
        html, sourcePath, rootPath, readAsset, revision,
        result: { html: "", unavailable: ["HTML 미리보기를 준비하지 못했습니다"] },
      });
    });
    return () => controller.abort();
  }, [html, sourcePath, rootPath, readAsset, revision]);
  useEffect(() => {
    if (!retainKeyboardFocus) return;
    let pending: ReturnType<typeof setTimeout> | undefined;
    const restore = () => {
      clearTimeout(pending);
      // iframe 키 이벤트는 부모로 버블링하지 않는다. 불투명 출처를 유지한 채
      // 미리보기의 키보드 포커스만 부모로 되돌린다.
      pending = setTimeout(() => {
        if (document.activeElement === frame.current) host.current?.focus({ preventScroll: true });
      }, 0);
    };
    window.addEventListener("blur", restore);
    frame.current?.addEventListener("focus", restore);
    const element = frame.current;
    return () => {
      clearTimeout(pending);
      window.removeEventListener("blur", restore);
      element?.removeEventListener("focus", restore);
    };
  }, [retainKeyboardFocus, prepare ? result : null]);
  // Mount with the complete document. The blank-then-populated frame lost layout on return from
  // source mode in the Chromium smoke test; preparing before mounting preserves that transition.
  const preview = prepare && !result ? null : <iframe ref={frame} srcDoc={prepare ? result!.html : html} sandbox="" title={title} className={`${className}${prepare ? " flex-1 min-h-0" : ""}`} />;
  if (prepare) return <div ref={host} tabIndex={retainKeyboardFocus ? -1 : undefined} className="w-full h-full flex min-h-0 flex-col outline-none">
    {!result && <div role="status" className="shrink-0 px-3 py-1 text-xs text-text-muted">HTML 미리보기를 준비하고 있습니다.</div>}
    {result && result.unavailable.length > 0 && <div role="status" className="shrink-0 px-3 py-1 text-xs text-text-muted" title={result.unavailable.join("\n")}>
      일부 스타일·이미지를 불러오지 못했습니다 ({result.unavailable.length}개).
    </div>}
    {preview}
  </div>;
  return retainKeyboardFocus
    ? <div ref={host} tabIndex={-1} className="w-full h-full outline-none">{preview}</div>
    : preview;
});

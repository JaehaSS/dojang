import type { ToolOutputContent } from "../../lib/ipc";
import { safeToolMediaUrl, safeToolResourceUri } from "../../lib/tool-output-media";

export function ToolOutput({ contents, onOpenLink }: { contents: ToolOutputContent[]; onOpenLink?: (link: string) => void }) {
  return <div className="max-w-[85%] space-y-2 rounded-md border border-border bg-bg px-2.5 py-2 text-xs">
    {contents.map((content, index) => {
      if (content.type === "text") return <pre key={index} className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words font-code text-text-muted">{content.text}</pre>;
      if (content.type === "image") {
        const url = safeToolMediaUrl(content.url, "image");
        return url ? <img key={index} src={url} alt="도구 이미지 결과" className="max-h-80 max-w-full rounded object-contain" /> : <p key={index} role="alert" className="text-text-secondary">표시할 수 없는 이미지 결과입니다.</p>;
      }
      if (content.type === "audio") {
        const url = safeToolMediaUrl(content.url, "audio");
        return url ? <audio key={index} controls preload="metadata" src={url} className="max-w-full" /> : <p key={index} role="alert" className="text-text-secondary">표시할 수 없는 음성 결과입니다.</p>;
      }
      const uri = safeToolResourceUri(content.uri);
      return uri && onOpenLink ? <button key={index} type="button" className="text-left text-primary-bright underline" onClick={() => onOpenLink(uri)}>{content.title || uri}</button> : <p key={index} role="alert" className="text-text-secondary">열 수 없는 리소스입니다.</p>;
    })}
  </div>;
}

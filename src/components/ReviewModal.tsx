import type { ReviewDetail } from "../lib/ipc";
import { Icon } from "./ide/icons";
import { Dialog, IconButton } from "./ui";

interface Props {
  detail: ReviewDetail;
  onClose: () => void;
}

export function ReviewModal({ detail, onClose }: Props) {
  const contentPreview = detail.content.length > 1000
    ? detail.content.slice(0, 1000) + "\n…(전체 길이: " + detail.content.length + " bytes)"
    : detail.content;

  return (
    <Dialog label="리뷰 상세 정보" onClose={onClose} className="w-full max-w-3xl max-h-[80vh] overflow-auto flex flex-col">
      <header className="flex items-center justify-between p-4 border-b border-border shrink-0">
        <h2 className="text-lg font-semibold">리뷰 상세 정보</h2>
        <IconButton label="닫기" onClick={onClose}>
          <Icon name="x" size={16} />
        </IconButton>
      </header>

      <div className="flex-1 overflow-auto p-4 space-y-6">
        {/* 콘텐츠 섹션 */}
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-medium uppercase tracking-wide text-text-muted">
            검토 대상 콘텐츠
          </h3>
          <div className="bg-bg border border-border rounded p-3 max-h-64 overflow-auto">
            <pre className="whitespace-pre-wrap font-code text-xs text-text break-words">
              {contentPreview}
            </pre>
          </div>
          <div className="text-xs text-text-muted">
            전체 크기: {detail.content.length} bytes
          </div>
        </section>

        {/* 프롬프트 섹션 */}
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-medium uppercase tracking-wide text-text-muted">
            리뷰 프롬프트
          </h3>
          <div className="bg-bg border border-border rounded p-3 max-h-64 overflow-auto">
            <pre className="whitespace-pre-wrap font-code text-xs text-text break-words">
              {detail.prompt_review}
            </pre>
          </div>
          {detail.prompt_synthesis && (
            <>
              <h3 className="text-sm font-medium uppercase tracking-wide text-text-muted mt-4">
                종합 프롬프트
              </h3>
              <div className="bg-bg border border-border rounded p-3 max-h-64 overflow-auto">
                <pre className="whitespace-pre-wrap font-code text-xs text-text break-words">
                  {detail.prompt_synthesis}
                </pre>
              </div>
            </>
          )}
        </section>

        {/* 모델 정보 섹션 */}
        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-medium uppercase tracking-wide text-text-muted">
            모델 정보
          </h3>
          <div className="bg-bg border border-border rounded overflow-hidden">
            <table className="w-full text-xs font-code">
              <thead>
                <tr className="border-b border-border bg-raised">
                  <th className="text-left px-3 py-2 text-text-muted font-medium">벤더</th>
                  <th className="text-left px-3 py-2 text-text-muted font-medium">모델</th>
                  <th className="text-left px-3 py-2 text-text-muted font-medium">커맨드</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {detail.model_info.map((info) => (
                  <tr key={info.vendor} className="hover:bg-raised/50">
                    <td className="px-3 py-2 text-text font-semibold">{info.vendor}</td>
                    <td className="px-3 py-2 text-text-secondary">{info.model || "(기본값)"}</td>
                    <td className="px-3 py-2 text-text-muted truncate max-w-xs" title={info.cmd}>
                      {info.cmd}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {detail.synthesis_model && (
            <div className="mt-3 p-3 bg-primary/5 border border-primary/20 rounded">
              <div className="text-xs text-text-muted mb-1">종합 의견 모델</div>
              <div className="grid grid-cols-3 gap-3 text-xs">
                <div>
                  <div className="text-text-muted">벤더</div>
                  <div className="font-semibold">{detail.synthesis_model.vendor}</div>
                </div>
                <div>
                  <div className="text-text-muted">모델</div>
                  <div className="font-semibold">
                    {detail.synthesis_model.model || "(기본값)"}
                  </div>
                </div>
                <div>
                  <div className="text-text-muted">커맨드</div>
                  <div className="font-code text-text-muted truncate">
                    {detail.synthesis_model.cmd}
                  </div>
                </div>
              </div>
            </div>
          )}
        </section>
      </div>
    </Dialog>
  );
}

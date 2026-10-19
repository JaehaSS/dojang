import { Component, useState, type ErrorInfo, type ReactNode } from "react";
import { copyText } from "../lib/clipboard";

/**
 * 렌더 예외 로그의 표식. 원격 세션이 "먹통"이 됐을 때 devtools 콘솔에서 이 한 줄만
 * 찾으면 되도록 다른 로그와 겹치지 않는 문자열을 쓴다.
 */
export const ERROR_BOUNDARY_MARKER = "[praxis:error-boundary]";

interface ErrorBoundaryProps {
  children: ReactNode;
}

interface ErrorBoundaryState {
  error: Error | null;
}

function Fallback({ error, onReset }: { error: Error; onReset: () => void }) {
  const [copied, setCopied] = useState(false);
  // 스택은 사용자가 개발자에게 그대로 넘기는 물건이다 — 손으로 긁게 두면 잘린 채로 온다.
  const copy = (): void => {
    void copyText(`${error.message}\n\n${error.stack ?? ""}`).catch(() => undefined);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-bg p-6 font-ui text-text">
      <div className="w-full max-w-2xl rounded-lg border border-dangerborder bg-raised p-4 shadow-xl">
        <h1 className="text-lg font-semibold text-status-failed">화면을 그리지 못했습니다</h1>
        <p className="mt-1 text-sm text-text-secondary">{error.message || String(error)}</p>
        <pre className="mt-3 max-h-64 select-text overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-bg p-2 font-code text-xs text-text-muted">
          {error.stack ?? "스택이 없습니다"}
        </pre>
        <div className="mt-3 flex gap-2">
          <button
            className="h-8 rounded-md bg-primary px-3 text-sm font-medium text-bg"
            onClick={onReset}
          >
            다시 시도
          </button>
          <button
            className="h-8 rounded-md border border-border px-3 text-sm text-text-secondary hover:border-border-strong hover:text-text"
            onClick={copy}
          >
            {copied ? "복사됨" : "스택 복사"}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * 렌더 중 예외가 하나라도 나면 React는 트리를 통째로 언마운트한다 — 그 결과가 빈 화면이고,
 * 빈 화면은 무엇이 터졌는지 아무것도 말해주지 않는다. 여기서 붙잡아 오류를 보이게 한다.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo): void {
    // componentStack은 error.stack에 없는 것을 준다 — 어느 컴포넌트에서 터졌는지.
    console.error(ERROR_BOUNDARY_MARKER, error, errorInfo.componentStack);
  }

  render(): ReactNode {
    const { error } = this.state;
    if (!error) return this.props.children;
    return <Fallback error={error} onReset={() => this.setState({ error: null })} />;
  }
}

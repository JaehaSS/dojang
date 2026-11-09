import { Icon } from "./icons";
import { IconButton } from "../ui";

interface Props {
  canSend: boolean;
  onInterrupt: () => void;
  onSend: () => void;
  sendLabel?: string;
}

export function AgentComposerActions({ canSend, onInterrupt, onSend, sendLabel }: Props) {
  return (
    <>
      <IconButton tone="danger" className="self-end" label="인터럽트" title="인터럽트 (Ctrl-C) — 진행 중 응답 중단" onClick={onInterrupt}>
        <Icon name="stop" size={16} />
      </IconButton>
      <button
        className={`shrink-0 self-end pb-0.5 ${
          canSend ? "text-primary-bright" : "text-text-muted"
        }`}
        disabled={!canSend}
        onClick={onSend}
        aria-label={sendLabel ?? "전송"}
        title={`${sendLabel ?? "전송"} (Enter)`}
      >
        {sendLabel ? <span className="text-xs whitespace-nowrap">{sendLabel}</span> : <Icon name="send" size={16} />}
      </button>
    </>
  );
}

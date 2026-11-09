import { useState } from "react";
import { designmodeCaptureEditor } from "../../lib/ipc";
import { readEditorCaptureTarget } from "../../lib/designmode/editor-capture-target";
import { pushCapture } from "../../lib/designmode/store";
import { Icon } from "./icons";

interface Props {
  taskId: number;
  available: boolean;
  onError: (message: string | null) => void;
}

export function EditorCaptureButton({ taskId, available, onError }: Props) {
  const [capturing, setCapturing] = useState(false);

  const capture = async (): Promise<void> => {
    const target = readEditorCaptureTarget(taskId);
    if (!target) {
      onError("에디터 탭에서 캡처할 파일을 먼저 열어주세요.");
      return;
    }
    onError(null);
    setCapturing(true);
    try {
      const record = await designmodeCaptureEditor(taskId, target);
      pushCapture(taskId, record);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setCapturing(false);
    }
  };

  return (
    <button
      className="shrink-0 px-2 py-1 rounded text-xs border border-border text-text-muted hover:text-text disabled:opacity-40"
      onClick={() => void capture()}
      disabled={!available || capturing}
      title={available ? "현재 에디터 화면을 세션에 첨부" : "에디터 탭에서 파일을 먼저 여세요"}
      aria-label="에디터 캡처"
    >
      <Icon name="desktop" size={14} /> {capturing ? "캡처 중…" : "에디터"}
    </button>
  );
}

import { useLayoutEffect, useRef, useState } from "react";
import { designmodeRemoveCapture, type DesignCaptureRecord } from "../../lib/ipc";
import {
  getCaptures,
  removeCapture,
  subscribeCaptures,
} from "../../lib/designmode/store";
import { captureImagePaths, formatCapturesPrompt } from "../../lib/designmode/prompt";
import { isLocalCapture } from "../../lib/designmode/selection-capture";

interface Options {
  taskId: number;
  host?: string;
  value: string;
  /** 전송이 성공했는지는 호출자가 안다 — 입력창을 비우는 것도 호출자의 몫이다. */
  onSend: (text: string, imagePaths: string[]) => void | boolean | Promise<void | boolean>;
}

export function useSessionCaptureAttachments({ taskId, host = "local", value, onSend }: Options) {
  const inflight = useRef(false);
  const [captures, setCaptures] = useState<DesignCaptureRecord[]>(() => host === "local" ? getCaptures(taskId) : []);

  useLayoutEffect(() => {
    if (host !== "local") {
      setCaptures([]);
      return;
    }
    setCaptures(getCaptures(taskId));
    return subscribeCaptures(taskId, setCaptures);
  }, [taskId, host]);

  const remove = (id: string): void => {
    removeCapture(taskId, id);
    // ⌘L 선택 첨부는 디스크에 남긴 것이 없다 — 지울 파일도 없으므로 IPC를 건너뛴다.
    if (!isLocalCapture(id)) designmodeRemoveCapture(taskId, id).catch(() => {});
  };

  /**
   * 입력값과 캡처를 합쳐 한 번에 넘긴다.
   *
   * 예전에는 합친 문자열을 `onChange`로 입력창에 되돌려 넣고, 그 값 변화를 `useEffect`가
   * 받아 전송했다. 전송이 `agentInput`이라는 공유 state를 지나는 2단계 비동기였고, 그 사이
   * 다른 호출자(에디터 창의 질문)가 끼어들면 사용자가 타이핑 중이던 내용을 덮어썼다.
   */
  const block = captures.length > 0 ? formatCapturesPrompt(captures) : "";
  const message = block ? `${value}${value.trim() ? "\n\n" : ""}${block}` : value;

  const send = async (): Promise<void> => {
    if (inflight.current) return;
    inflight.current = true;
    try {
      const accepted = await onSend(message, captureImagePaths(captures));
      if (accepted === false) return;
      // Consume this submission only. Captures added while awaiting admission survive.
      for (const capture of captures) removeCapture(taskId, capture.id);
    } catch {
      // Caller presents the error; attachments remain available for the same request retry.
    } finally {
      inflight.current = false;
    }
  };

  return { captures, message, remove, send };
}

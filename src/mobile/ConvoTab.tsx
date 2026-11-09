import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import {
  appendItems,
  isTurnComplete,
  parseConvoEvent,
  toItems,
  type MobileConvoItem,
} from "./convo";
import { Empty, Spinner } from "./primitives";
import { Icon } from "../components/ide/icons";

// 대화 탭 — 스트림 표시 + 후속 지시 전송. (설계 0013 §10 M2)
// 마크다운은 렌더하지 않는다. 번들에 react-markdown을 끌어오는 값에 비해 폰에서 얻는
// 이득이 작고, 평문 그대로도 읽힌다.

const BOTTOM_SLACK_PX = 64;

function Bubble({ item }: { item: MobileConvoItem }) {
  switch (item.role) {
    case "user":
      return (
        <div className="ml-8 rounded-lg border border-border bg-raised px-3 py-2 text-sm text-text">
          {item.text}
        </div>
      );
    case "text":
      return <div className="whitespace-pre-wrap text-sm text-text">{item.text}</div>;
    case "tool":
      return (
        <div className="font-code text-xs text-text-muted">
          → {item.name}
          {item.summary ? ` ${item.summary}` : ""}
        </div>
      );
    case "result":
      return (
        <div className={`flex items-start gap-1 font-code text-xs ${item.error ? "text-status-failed" : "text-text-muted"}`}>
          <span className="mt-0.5 inline-flex shrink-0"><Icon name={item.error ? "x" : "check"} size={12} /></span>
          <span className="min-w-0">{item.summary}</span>
        </div>
      );
    case "error":
      return (
        <div className="rounded-md border border-dangerborder bg-dangerbg px-3 py-2 text-sm text-text">
          {item.text}
        </div>
      );
    case "meta":
      return (
        <div className="text-xs text-text-muted">
          {item.turns}턴 · ${item.cost.toFixed(4)}
        </div>
      );
  }
}

export function ConvoTab({
  id,
  canSend,
  blockedReason,
}: {
  id: number;
  /** 지금 후속 메시지를 받을 수 있는지 — Runner의 조건과 같다(actions.followupAvailability). */
  canSend: boolean;
  blockedReason?: string;
}) {
  const [items, setItems] = useState<MobileConvoItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const stickToBottom = useRef(true);

  useEffect(() => {
    let cancelled = false;
    let after = 0;
    setItems(null);
    setError(null);

    let chain = Promise.resolve();
    const load = () => {
      chain = chain.then(async () => {
        try {
          for (;;) {
            if (cancelled) return;
            const rows = await api.taskOutput(id, after);
            if (cancelled || rows.length === 0) return;
            after = rows[rows.length - 1].sequence;
            const events = rows
              .map((row) => parseConvoEvent(row.data))
              .filter((event): event is NonNullable<typeof event> => event !== null);
            if (events.length === 0) continue;
            const next = toItems(events);
            if (next.length > 0) {
              setItems((previous) => appendItems(previous ?? [], next));
            }
            if (isTurnComplete(events)) setBusy(false);
          }
        } catch (cause: unknown) {
          if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
        }
      });
    };
    load();
    // 첫 조회가 비어 있어도 로딩 상태에 머물지 않게 빈 배열로 확정한다.
    void chain.then(() => {
      if (!cancelled) setItems((previous) => previous ?? []);
    });

    const stop = api.subscribeEvents(
      0,
      (event) => {
        if (event.task_id === id && event.kind === "output") load();
      },
      () => {},
    );
    return () => {
      cancelled = true;
      stop();
    };
  }, [id]);

  useEffect(() => {
    const container = containerRef.current;
    if (container && stickToBottom.current) container.scrollTop = container.scrollHeight;
  }, [items]);

  const send = async () => {
    const message = draft.trim();
    if (!message || busy || !canSend) return;
    setBusy(true);
    setSendError(null);
    // 낙관적으로 먼저 그린다. 서버가 durable user 이벤트를 되돌려주면 appendItems가 합친다.
    setItems((previous) => [...(previous ?? []), { role: "user", text: message }]);
    setDraft("");
    stickToBottom.current = true;
    try {
      await api.taskMessage(id, message);
    } catch (cause: unknown) {
      setSendError(cause instanceof Error ? cause.message : String(cause));
      setBusy(false);
    }
  };

  if (error) return <Empty>대화를 불러오지 못했습니다. {error}</Empty>;
  if (!items) return <Spinner label="대화를 불러오는 중" />;

  return (
    <div className="flex h-[60vh] flex-col">
      <div
        ref={containerRef}
        onScroll={(event) => {
          const el = event.currentTarget;
          stickToBottom.current =
            el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_SLACK_PX;
        }}
        className="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-3"
      >
        {items.length === 0 ? (
          <div className="py-8 text-center text-sm text-text-muted">아직 대화가 없습니다.</div>
        ) : (
          items.map((item, index) => <Bubble key={index} item={item} />)
        )}
      </div>

      {sendError ? (
        <div className="border-t border-dangerborder bg-dangerbg px-4 py-2 text-xs text-text">
          {sendError}
        </div>
      ) : null}

      {/* 왜 못 보내는지 말한다 — 버튼만 비활성이면 사용자는 앱이 고장난 줄 안다. */}
      {!canSend && blockedReason ? (
        <div className="border-t border-border bg-raised px-4 py-2 text-xs text-text-muted">
          {blockedReason}
        </div>
      ) : null}

      <div className="flex items-end gap-2 border-t border-border bg-surface px-3 py-2">
        <textarea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          rows={1}
          placeholder={canSend ? "후속 지시" : "지금은 보낼 수 없습니다"}
          className="max-h-32 min-h-[44px] flex-1 resize-none rounded-lg border border-border bg-bg px-3 py-2 text-sm text-text outline-none"
        />
        <button
          type="button"
          onClick={() => void send()}
          disabled={busy || !canSend || draft.trim() === ""}
          className="min-h-[44px] shrink-0 rounded-lg bg-primary px-4 text-sm text-black disabled:opacity-40"
        >
          {busy ? "…" : "전송"}
        </button>
      </div>
    </div>
  );
}

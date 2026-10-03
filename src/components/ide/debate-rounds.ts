import { DEBATE_SEATS, type DebateEndReason, type DebateSpeaker } from "../../lib/ipc";
import { eventToItems, type ConvoEventLike, type ConvoItem } from "./ConversationView";

/** 토론 이벤트 — 저장 이력·라이브 스트림 공용 형태에 발화자만 얹은 것. */
export type DebateEventLike = ConvoEventLike & {
  speaker?: DebateSpeaker;
  reason?: DebateEndReason;
};

/** 라운드 하나 — 같은 행에 놓이는 자리별 발화와, 어느 면에도 속하지 않는 공통 영역. */
export interface DebateRound {
  /** 사용자 발화마다 1부터 다시 센다 — 백엔드의 라운드 상한이 그 단위로 걸린다. */
  no: number;
  /** 자리별 발화, `DEBATE_SEATS` 순서. 자리 수와 무관하게 항상 셋이고 뷰가 앞의 n개만 그린다. */
  panes: ConvoItem[][];
  /** 이 라운드를 연 사용자 발화. */
  user?: string;
  /** 발화자 미상 이벤트 — 어느 면으로도 접지 않는다(오귀속 금지). */
  common: ConvoItem[];
}

export interface DebateTranscript {
  /** 첫 라운드 이전의 이벤트 전부. 토론 이전 대화가 여기 남는다. */
  preamble: ConvoItem[];
  rounds: DebateRound[];
  /** 마지막 `debate_ended`. 화면 상태는 `running | ended(reason)` 둘뿐이다. */
  ended?: DebateEndReason;
}

/** 자리 순서상 위치. 알 수 없는 발화자는 -1 — 호출부가 미상으로 다룬다. */
export const seatIndex = (speaker: DebateSpeaker | undefined): number =>
  speaker == null ? -1 : DEBATE_SEATS.indexOf(speaker);

/**
 * 이 이벤트로 시퀀스가 끝났는가 — 컴포저 잠금(busy)을 푸는 유일한 기준.
 *
 * 토론은 면마다 `result`를 낸다. 그것으로 풀면 좌측이 끝나는 순간 컴포저가 열려 우측이 도는
 * 중에 `convo_send`가 겹친다. 시퀀스를 닫는 것은 `debate_ended`뿐이고, 토론이 아닌 턴은
 * `speaker` 없는 `result` 하나로 끝난다.
 */
export const endsSequence = (ev: DebateEventLike): boolean =>
  ev.kind === "debate_ended" || (ev.kind === "result" && ev.speaker == null);

/**
 * 이벤트 배열 → 라운드 행 grid의 재료.
 *
 * 규칙 셋뿐이다. ① `speaker`가 없으면 공통 영역이다 — 없다는 것은 좌측이 아니라 **미상**이라
 * 한쪽 면으로 접으면 오귀속이 된다. ② 자리 순서가 되감기면(앞 자리가 뒤 자리 다음에 오면)
 * 새 라운드다 — 2자면 우측 다음 좌측, 3자면 셋째 다음 좌측이다. ③ 사용자 발화는
 * 라운드 번호를 1로 되돌린다 — 상한이 발화 한 건의 라운드 시퀀스에 걸리므로 이어 세면
 * `라운드 4/3`이 나온다.
 */
export function debateRounds(events: readonly DebateEventLike[]): DebateTranscript {
  const preamble: ConvoItem[] = [];
  const rounds: DebateRound[] = [];
  let current: DebateRound | null = null;
  let lastSeat = -1;
  let base = 0;
  let ended: DebateEndReason | undefined;

  const open = (user?: string): DebateRound => {
    const round: DebateRound = { no: rounds.length - base + 1, panes: DEBATE_SEATS.map(() => []), common: [], user };
    rounds.push(round);
    current = round;
    lastSeat = -1;
    return round;
  };

  const all = events as ConvoEventLike[];
  events.forEach((ev, index) => {
    if (ev.kind === "debate_ended") {
      ended = ev.reason ?? "aborted";
      return;
    }
    const items = eventToItems(ev, index, all);
    const seat = seatIndex(ev.speaker);
    if (seat < 0) {
      if (ev.kind === "user") {
        // 새 발화는 새 시퀀스다 — 번호를 되돌리고 직전 배너를 지운다.
        base = rounds.length;
        ended = undefined;
        open(ev.text ?? "");
        return;
      }
      (current ? current.common : preamble).push(...items);
      return;
    }
    let round = current;
    if (round == null || seat < lastSeat) round = open();
    round.panes[seat].push(...items);
    lastSeat = seat;
  });

  return { preamble, rounds, ended };
}

/** 마지막 라운드에서 발화가 있는 가장 뒤 자리. 없으면 0(좌측). */
function lastSpokenSeat(transcript: DebateTranscript): number {
  const last = transcript.rounds[transcript.rounds.length - 1];
  if (!last) return 0;
  for (let seat = last.panes.length - 1; seat > 0; seat -= 1) if (last.panes[seat].length > 0) return seat;
  return 0;
}

/** 지금 도는 면 — 커서는 하나뿐이라 마지막 라운드에서 마지막으로 말한 쪽에 둔다. */
export function activeSide(transcript: DebateTranscript): DebateSpeaker {
  return DEBATE_SEATS[lastSpokenSeat(transcript)];
}

/** 합의 결론 — 마지막 라운드에서 마지막으로 나온 발화 본문. `결론 복사`가 넘기는 것. */
export function consensusText(transcript: DebateTranscript): string {
  const last = transcript.rounds[transcript.rounds.length - 1];
  if (!last) return "";
  const text = [...last.panes[lastSpokenSeat(transcript)]].reverse().find((item) => item.role === "text");
  return text?.role === "text" ? text.text : "";
}

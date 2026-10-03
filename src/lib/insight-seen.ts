// 아직 보지 않은 발견 카드 — 사이드바 인사이트 항목의 점 상태(설계 2026-09-28 §4, 슬라이스 T5).
//
// 예전 회고 신선도 점(`retro-seen.ts`, #565에서 회고와 함께 폐기)과 같은 자리이지만 단위가
// 다르다. "보았다"의 단위는 카드 `key`가 아니라 **(signal, repo) 쌍**이다 — key는 이동 구간의
// 시작일을 담아 매일 바뀌므로(설계 §5.3), key로 남기면 어제 본 카드가 오늘 또 "새 발견"이 된다.
// 백엔드 무시(`insight_dismissals`, §6.4)도 같은 쌍 단위를 쓴다.
//
// TTL은 무시와 같은 7일이다 — 그 뒤엔 같은 신호가 다시 떠도 새로 발견한 것으로 되살아난다.
// localStorage인 이유도 무시·회고와 같다: 이것은 기기별 UI 상태이지 공유할 도메인 사실이 아니다.

const STORAGE_KEY = "praxis:insight-seen";
const TTL_MS = 7 * 24 * 60 * 60 * 1000;

/** 카드에서 seen 판정에 쓰는 최소 필드. `SignalCard`의 부분집합이라 그대로 넘길 수 있다. */
export interface SeenIdentity {
  signal: string;
  repo: string | null;
}

interface SeenRecord {
  signal: string;
  repo: string;
  seenAt: number;
}

type SeenStore = Record<string, SeenRecord>;

/** 저장소 없는 신호는 빈 문자열로 접는다 — 백엔드 `insight_dismissals`와 같은 관례(§6.4). */
function pairId(signal: string, repo: string | null): string {
  return `${signal}\u0000${repo ?? ""}`;
}

function load(): SeenStore {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    return parsed && typeof parsed === "object" ? (parsed as SeenStore) : {};
  } catch {
    return {};
  }
}

function save(store: SeenStore): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch {
    // private mode 등 storage 거부 시 현재 세션 상태만 유지한다(praxis:diff-viewed와 같은 방침).
  }
}

/** 이 (signal, repo) 쌍을 최근 7일 안에 보았는가. */
export function isSeen(signal: string, repo: string | null, now: number = Date.now()): boolean {
  const rec = load()[pairId(signal, repo)];
  return rec != null && now - rec.seenAt < TTL_MS;
}

/** 카드 목록 중 아직(또는 TTL이 지나 다시) 보지 않은 것의 수. */
export function countUnseen(cards: SeenIdentity[], now: number = Date.now()): number {
  const store = load();
  let count = 0;
  for (const c of cards) {
    const rec = store[pairId(c.signal, c.repo)];
    if (rec == null || now - rec.seenAt >= TTL_MS) count++;
  }
  return count;
}

/**
 * 지금 화면에 보이는 카드를 전부 "보았다"로 남긴다. 인사이트 화면을 여는 것 자체가
 * 읽음 처리다(설계 §4) — 별도의 "읽음" 버튼은 없다.
 *
 * 저장 김에 TTL이 지난 옛 기록을 걷어낸다 — 그러지 않으면 저장소가 무한히 자란다.
 */
export function markSeen(cards: SeenIdentity[], now: number = Date.now()): void {
  const store = load();
  for (const [id, rec] of Object.entries(store)) {
    if (now - rec.seenAt >= TTL_MS) delete store[id];
  }
  if (cards.length === 0) {
    save(store);
    return;
  }
  for (const c of cards) {
    store[pairId(c.signal, c.repo)] = { signal: c.signal, repo: c.repo ?? "", seenAt: now };
  }
  save(store);
}

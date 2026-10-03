export interface ConversationScrollPosition {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}

const FOLLOW_THRESHOLD_PX = 48;

/** 사용자가 대화 하단을 보고 있을 때만 스트리밍 출력을 계속 따라간다. */
export function isConversationNearBottom(
  position: ConversationScrollPosition,
  threshold: number = FOLLOW_THRESHOLD_PX,
): boolean {
  const remaining = position.scrollHeight - position.scrollTop - position.clientHeight;
  return remaining <= threshold;
}

/** 보던 자리 — 뷰포트 맨 위에 걸친 항목과, 그 항목의 윗변이 뷰포트 윗변에서 떨어진 거리. */
export interface ConversationScrollAnchor {
  node: Element;
  offset: number;
}

/**
 * 뷰포트 맨 위에 걸친 직계 자식을 찾는다. 항목은 위에서 아래로 쌓이므로 아랫변이 단조 증가한다 —
 * 스크롤마다 부르는 함수라 선형 대신 이분 탐색으로 찾는다.
 */
export function captureScrollAnchor(container: HTMLElement): ConversationScrollAnchor | null {
  const children = container.children;
  const top = container.getBoundingClientRect().top;
  let lo = 0;
  let hi = children.length - 1;
  let found: Element | null = null;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const node = children[mid];
    if (node.getBoundingClientRect().bottom > top) {
      found = node;
      hi = mid - 1;
    } else {
      lo = mid + 1;
    }
  }
  return found ? { node: found, offset: found.getBoundingClientRect().top - top } : null;
}

/** 같은 항목을 같은 거리에 다시 놓는다. 그 항목이 이 목록을 떠났으면 건드리지 않는다. */
export function restoreScrollAnchor(container: HTMLElement, anchor: ConversationScrollAnchor | null): void {
  if (!anchor || anchor.node.parentElement !== container) return;
  const drift = anchor.node.getBoundingClientRect().top - container.getBoundingClientRect().top - anchor.offset;
  if (drift !== 0) container.scrollTop += drift;
}

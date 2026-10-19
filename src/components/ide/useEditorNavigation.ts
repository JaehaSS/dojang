import { useCallback, useEffect, useRef, useState } from "react";

import type { LspTarget } from "../../lib/ipc";
import {
  currentNavigation,
  emptyNavigation,
  pushNavigation,
  type NavigationLocation,
  type NavigationState,
} from "../../lib/editor-navigation";

export interface EditorNavigationIdentity {
  host: string;
  taskId: number | null;
  windowId: string;
  workspaceKey?: string;
}
export type OpenTargetOutcome = "opened" | "external" | "failed";
export interface NavigationReveal extends NavigationLocation {
  navId: number;
  epoch: number;
  restore: boolean;
}

interface Options {
  identity: EditorNavigationIdentity | null;
  openTarget: (target: LspTarget) => Promise<OpenTargetOutcome>;
  onError: (message: string) => void;
}

interface Pending {
  id: number;
  epoch: number;
  origin: NavigationLocation | null;
  target: NavigationLocation;
  next: NavigationState;
  timer: number;
}

const asTarget = (location: NavigationLocation): LspTarget => ({
  path: location.path,
  abs_path: location.path,
  line: location.line,
  column: location.column,
  external: false,
});

export function useEditorNavigation({
  identity,
  openTarget,
  onError,
}: Options) {
  const [history, setHistory] = useState<NavigationState>(emptyNavigation);
  const [reveal, setReveal] = useState<NavigationReveal | null>(null);
  const pendingRef = useRef<Pending | null>(null);
  const epochRef = useRef(0);
  const idRef = useRef(0);
  const identityKey =
    identity == null
      ? ""
      : `${identity.host}:${identity.taskId}:${identity.windowId}:${identity.workspaceKey ?? ""}`;
  const identityKeyRef = useRef(identityKey);
  if (identityKeyRef.current !== identityKey) {
    identityKeyRef.current = identityKey;
    epochRef.current += 1;
    const pending = pendingRef.current;
    if (pending) globalThis.clearTimeout(pending.timer);
    pendingRef.current = null;
  }

  const clearPending = useCallback(() => {
    const pending = pendingRef.current;
    if (pending) globalThis.clearTimeout(pending.timer);
    pendingRef.current = null;
  }, []);

  useEffect(() => {
    epochRef.current += 1;
    clearPending();
    setHistory(emptyNavigation());
    setReveal(null);
    return () => {
      epochRef.current += 1;
      clearPending();
    };
  }, [identity?.host, identity?.taskId, identity?.windowId, identity?.workspaceKey, clearPending]);

  const revealAfterOpen = useCallback(
    (
      id: number,
      epoch: number,
      origin: NavigationLocation | null,
      target: NavigationLocation,
      next: NavigationState,
      restore: boolean,
    ) => {
      if (epoch !== epochRef.current) return;
      clearPending();
      const timer = globalThis.setTimeout(() => {
        const pending = pendingRef.current;
        if (
          !pending ||
          pending.id !== id ||
          pending.epoch !== epoch ||
          epoch !== epochRef.current
        )
          return;
        pendingRef.current = null;
        setReveal(null);
        onError("코드 위치로 이동하지 못했습니다");
      }, 5_000);
      pendingRef.current = { id, epoch, origin, target, next, timer };
      setReveal({ ...target, navId: id, epoch, restore });
    },
    [clearPending, onError],
  );

  const openThenReveal = useCallback(
    async (
      id: number,
      epoch: number,
      origin: NavigationLocation | null,
      target: NavigationLocation,
      next: NavigationState,
      openRequest = asTarget(target),
      restore = true,
    ) => {
      try {
        const outcome = await openTarget(openRequest);
        if (
          epoch !== epochRef.current ||
          id !== idRef.current ||
          outcome !== "opened"
        )
          return false;
        revealAfterOpen(id, epoch, origin, target, next, restore);
        return true;
      } catch {
        if (epoch === epochRef.current && id === idRef.current)
          onError("파일을 열지 못했습니다");
        return false;
      }
    },
    [onError, openTarget, revealAfterOpen],
  );

  const navigate = useCallback(
    (target: LspTarget, origin: NavigationLocation) => {
      if (!identity) return;
      if (target.external || target.path == null) {
        void openTarget(target).catch(() => onError("파일을 열지 못했습니다"));
        return;
      }
      const id = ++idRef.current;
      const epoch = epochRef.current;
      clearPending();
      setReveal(null);
      const destination = {
        ...origin,
        path: target.path,
        line: target.line,
        column: target.column,
        scrollTop: 0,
        scrollLeft: 0,
      };
      const next = pushNavigation(pushNavigation(history, origin), destination);
      void openThenReveal(id, epoch, origin, destination, next, target, false);
    },
    [clearPending, history, identity, onError, openTarget, openThenReveal],
  );

  const onRevealed = useCallback(
    (
      ack:
        | (Pick<NavigationReveal, "navId" | "epoch"> &
            Partial<NavigationLocation>)
        | undefined,
    ) => {
      const pending = pendingRef.current;
      if (
        !pending ||
        !ack ||
        pending.id !== ack.navId ||
        pending.epoch !== ack.epoch ||
        ack.navId !== idRef.current ||
        ack.epoch !== epochRef.current
      )
        return;
      clearPending();
      setReveal(null);
      const actual =
        ack.path == null ? pending.target : { ...pending.target, ...ack };
      setHistory({
        ...pending.next,
        entries: pending.next.entries.map((entry, index) =>
          index === pending.next.index ? actual : entry,
        ),
      });
    },
    [clearPending],
  );

  const move = useCallback(
    (direction: -1 | 1) => {
      if (!identity) return;
      let index = history.index + direction;
      if (index < 0 || index >= history.entries.length) return;
      const id = ++idRef.current;
      const epoch = epochRef.current;
      clearPending();
      setReveal(null);
      void (async () => {
        while (
          index >= 0 &&
          index < history.entries.length &&
          epoch === epochRef.current &&
          id === idRef.current
        ) {
          const next = { entries: history.entries, index };
          const target = currentNavigation(next);
          if (target && (await openThenReveal(id, epoch, null, target, next))) return;
          if (epoch !== epochRef.current || id !== idRef.current) return;
          onError("이동 위치의 파일을 열지 못해 건너뛰었습니다");
          index += direction;
        }
      })();
    },
    [clearPending, history, identity, openThenReveal],
  );

  return {
    reveal: reveal?.epoch === epochRef.current ? reveal : null,
    navigate,
    onRevealed,
    back: () => move(-1),
    forward: () => move(1),
    canBack: history.index > 0,
    canForward:
      history.index >= 0 && history.index < history.entries.length - 1,
  };
}

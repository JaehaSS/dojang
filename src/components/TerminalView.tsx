import { useEffect, useMemo, useRef } from "react";
import type { HostId } from "../lib/transport";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { taskWrite, taskResize, taskPtyReplay } from "../lib/ipc";
import { getTransport } from "../lib/transport";
import { RunnerTransport } from "../lib/transport/runner";
import { attachPtyStream, ptyEventNames } from "../lib/pty-stream";
import { findTerminalFileLinks } from "../lib/terminal-file-links";
import { xtermTheme } from "../lib/themes";
import { useTheme } from "../lib/use-theme";
import { tryLoadWebgl, disposeWebgl, disposeTerminal } from "../lib/xterm-webgl";

/**
 * xterm은 CSS 변수를 읽지 못해 실제 색 값을 받아야 한다. 테마가 8종이 된 뒤로는
 * 라이트/다크 이진값으로 팔레트를 고를 수 없어, prop 대신 테마 스토어를 직접 구독한다.
 */
export function useXtermTheme() {
  const theme = useTheme();
  return useMemo(() => xtermTheme(theme), [theme]);
}

/** 선택된 Task의 라이브 PTY 페인. 이벤트를 task id로 필터링한다.
 *  readOnly: 출력만 렌더(입력은 하단 컴포저 전용) — stdin 비활성 + onData 미연결. */
export function TerminalView({
  taskId,
  host: taskHost,
  readOnly = false,
  onOpenLink,
  codeFontFamily = "'JetBrains Mono','Fira Code',ui-monospace,monospace",
  codeFontSize = 13,
}: {
  taskId: number;
  /** 작업이 사는 호스트 — PTY·리사이즈는 그 머신의 것이다. */
  /** 작업이 사는 호스트 — PTY·리사이즈는 그 머신의 것이다. (지역 `host`는 DOM 컨테이너다) */
  host: HostId;
  readOnly?: boolean;
  /** 에이전트가 OSC 8로 출력한 파일/웹 링크 열기. */
  onOpenLink?: (link: string) => void;
  /** 완성된 코드 폰트 스택 문자열(fonts.ts codeFontStack 결과). 기본값 = 기존 하드코딩 값. */
  codeFontFamily?: string;
  /** 코드 폰트 크기(px). 기본값 = 기존 하드코딩 값. */
  codeFontSize?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const theme = useXtermTheme();
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const openLinkRef = useRef(onOpenLink);
  openLinkRef.current = onOpenLink;

  useEffect(() => {
    const host = ref.current;
    if (!host) return;
    let disposed = false;

    const term = new Terminal({
      fontFamily: codeFontFamily,
      fontSize: codeFontSize,
      cursorBlink: !readOnly,
      disableStdin: readOnly || getTransport(taskHost).kind === "remote",
      theme: themeRef.current,
      linkHandler: onOpenLink
        ? {
            allowNonHttpProtocols: true,
            activate: (_event, link) => openLinkRef.current?.(link),
          }
        : null,
    });
    termRef.current = term;
    // OSC 8로 오지 않은 평문 경로도 클릭 대상으로 만든다 — 에이전트 출력은 대개 평문이다.
    if (onOpenLink) {
      term.registerLinkProvider({
        provideLinks: (y, callback) => {
          callback(
            findTerminalFileLinks(term.buffer.active, y).map((link) => ({
              range: link.range,
              text: link.text,
              activate: () => openLinkRef.current?.(link.text),
            })),
          );
        },
      });
    }
    const fit = new FitAddon();
    fitRef.current = fit;
    term.loadAddon(fit);
    term.open(host);
    const webgl = tryLoadWebgl(term);

    // 레이아웃 완료 후 측정 (마운트 직후 host 높이가 0일 수 있음).
    const doFit = () => {
      if (disposed) return;
      try {
        fit.fit();
        if (getTransport(taskHost).kind === "local") taskResize(taskId, term.cols, term.rows).catch(() => {});
      } catch {
        /* 아직 레이아웃 전 — 다음 RO에서 재시도 */
      }
    };
    // 입력을 받는 터미널은 앱 컴포저 없이 이것이 곧 입력창이다 — 작업을 열면 바로 칠 수 있게 포커스한다.
    const interactive = !readOnly && getTransport(taskHost).kind !== "remote";
    requestAnimationFrame(() => {
      doFit();
      if (interactive && !disposed) term.focus();
    });

    const transport = getTransport(taskHost);
    // replay(스크롤백)를 선전송한 뒤 라이브 스트림을 구독한다 — 유실/중복 방지 순서 고정.
    // remote 전송은 이 커맨드가 없으므로 replay를 빈 문자열로 두고, 아래에서 작업 단위
    // `taskOutput`으로 이력을 복원한다(이벤트 스트림은 복원 채널이 되지 못한다 — 상세는 거기 주석).
    let stopStream: (() => void) | undefined;
    attachPtyStream({
      id: taskId,
      ...ptyEventNames("pty", taskId),
      fetchReplay: () => (transport.kind === "local" ? taskPtyReplay(taskId) : Promise.resolve("")),
      onData: (bytes) => {
        // 디스포즈 이후 도착하는 이벤트가 죽은 터미널에 write 하지 않도록 가드(#unlisten 레이스).
        if (!disposed) term.write(bytes);
      },
      onExit: (code) => {
        if (!disposed) term.writeln(`\r\n\x1b[90m[process exited: ${code}]\x1b[0m`);
      },
    }).then((stop) => {
      if (disposed) stop();
      else stopStream = stop;
    });
    const remote = transport instanceof RunnerTransport ? transport : null;
    let stopRemote: (() => void) | undefined;
    if (remote) {
      // 이력은 반드시 작업 단위 `taskOutput`으로 읽는다. 이벤트 스트림으로 복원할 수 없는
      // 이유는 구독 커서가 **endpoint당 하나**이고 localStorage에 영속되기 때문이다 —
      // `subscribeEvents(0, …)`로 "처음부터"를 요청해도 `Math.max(after, 저장된 커서)`가
      // 커서를 택하고, 그보다 앞선 이벤트는 전부 버려진다. 커서는 이벤트를 받을 때마다
      // 전진하므로 이미 끝난 작업의 출력은 **항상** 그 아래에 있어 한 줄도 오지 않는다.
      // (대화 모드가 App.tsx에서 같은 방식으로 이력을 읽는 것도 같은 이유다.)
      let after = 0;
      // 직렬화 체인 — output 이벤트 burst에 동시 fetch로 같은 구간을 두 번 write 방지.
      let chain = Promise.resolve();
      const drain = () => {
        chain = chain.then(async () => {
          try {
            // task별 필터 endpoint라 빈 응답 = 더 없음. 응답 상한 초과분은 반복 조회.
            for (;;) {
              if (disposed) return;
              const rows = await remote.taskOutput(taskId, after);
              if (disposed || rows.length === 0) return;
              after = rows[rows.length - 1].sequence;
              for (const row of rows) term.write(row.data);
            }
          } catch {
            /* 연결 오류 — 다음 output 이벤트/재선택에서 재시도 */
          }
        });
      };
      drain();
      stopRemote = remote.subscribeEvents(
        0,
        (event) => {
          if (disposed || event.task_id !== taskId) return;
          // 이벤트는 "새 출력이 생겼다"는 신호로만 쓴다 — 본문은 위 drain이 갖는다.
          if (event.kind === "output") drain();
          if (event.kind === "completed") term.writeln("\r\n\x1b[90m[process exited]\x1b[0m");
        },
        () => {},
      );
    }
    // 읽기전용 출력 뷰에서는 입력을 PTY로 보내지 않는다(입력은 하단 컴포저 전용).
    const onData = readOnly || remote
      ? null
      : term.onData((d) => {
          taskWrite(taskId, d).catch(() => {});
        });

    // 드래그 리사이즈 동안 fit/PTY가 초당 수십 번 호출되지 않게 디바운스.
    let rt: number | undefined;
    const ro = new ResizeObserver(() => {
      if (rt) clearTimeout(rt);
      rt = window.setTimeout(doFit, 60);
    });
    ro.observe(host);

    return () => {
      disposed = true;
      if (rt) clearTimeout(rt);
      ro.disconnect();
      onData?.dispose();
      stopStream?.();
      stopRemote?.();
      disposeWebgl(webgl);
      disposeTerminal(term);
      termRef.current = null;
      fitRef.current = null;
    };
  }, [taskHost, taskId, readOnly]);

  // 테마 전환 시 살아있는 터미널에 즉시 반영.
  useEffect(() => {
    if (termRef.current) termRef.current.options.theme = theme;
  }, [theme]);

  // 폰트 변경 시 살아있는 터미널에 즉시 반영 + 재측정(cols/rows 변동 → doFit 재호출 패턴).
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.fontFamily = codeFontFamily;
    term.options.fontSize = codeFontSize;
    try {
      fitRef.current?.fit();
      if (getTransport(taskHost).kind === "local") taskResize(taskId, term.cols, term.rows).catch(() => {});
    } catch {
      /* 아직 레이아웃 전 — 다음 리사이즈에서 재계산 */
    }
  }, [codeFontFamily, codeFontSize, taskHost, taskId]);

  // p-1 제거: 패딩은 FitAddon의 cols/rows 계산을 어긋나게 함.
  return <div ref={ref} className="flex-1 min-h-0 bg-bg" />;
}

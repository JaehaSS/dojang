import { invoke } from "@tauri-apps/api/core";

/**
 * 텍스트를 시스템 클립보드에 쓴다.
 *
 * 앱 안에서는 네이티브 명령이 먼저다. WKWebView의 `navigator.clipboard.writeText`는 사용자
 * 활성화가 없다고 판단하면 `NotAllowedError`로 거절한다 — 토론의 `결론 복사`가 그렇게 실패했다.
 * 네이티브 쓰기가 없는 플랫폼과 앱 밖(테스트·웹 클라이언트)에서만 웹 API로 간다.
 *
 * `@tauri-apps/api`의 `isTauri`를 쓰지 않는 것은 테스트가 그 모듈을 통째로 mock하기 때문이다.
 */
export async function copyText(text: string): Promise<void> {
  if ((globalThis as { isTauri?: boolean }).isTauri) {
    try {
      return await invoke("clipboard_write_text", { text });
    } catch {
      // 네이티브가 없는 플랫폼 — 웹 API로 넘어간다.
    }
  }
  const clipboard = navigator.clipboard;
  if (!clipboard) throw new Error("클립보드를 쓸 수 없는 환경입니다");
  await clipboard.writeText(text);
}

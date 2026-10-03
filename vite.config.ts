import { defineConfig } from "vite";
// `test` 키를 vite 설정 타입에 얹는 것은 이 import의 부수효과다 — 지우면 타입이 깨진다.
import { configDefaults } from "vitest/config";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
// 두 개의 산출물을 만든다.
//   기본  : index.html   → dist/        (Tauri frontendDist, 데스크톱 셸)
//   mobile: mobile.html  → dist-mobile/ (praxis-runner가 /m/ 로 서빙, rust-embed 내장)
// 모바일은 base="/m/"라 자산 경로가 /m/assets/*로 나가고, publicDir도 분리해
// 데스크톱 번들에 SW·manifest가 섞이지 않게 한다. (설계 0013 §5.2·§9)
export default defineConfig(async ({ mode }) => {
  const isMobile = mode === "mobile";

  return {
    plugins: [react()],

    // 저장소 **안에** 사는 worktree를 훑지 않는다. `.claude/worktrees`·`.praxis/worktrees`는
    // 다른 시점의 소스 트리 전체를 담고 있어, 그 안의 테스트는 그때의 코드에 맞춰 통과하던
    // 것이다. 지금 트리에서 돌리면 실패로 나오고, 그 실패는 방금 만든 회귀처럼 읽힌다.
    // 수집 비용도 크다 — worktree 9개가 있던 시점의 `src/components/ide`는 90초, 걷어낸
    // 뒤에는 4초였다. exclude는 기본값을 덮어쓰므로 configDefaults를 펼쳐 얹는다.
    test: {
      // 여러 작업 트리의 검증이 겹쳐도 각 실행이 CPU 수만큼 워커를 띄우지 않는다.
      minWorkers: 1,
      maxWorkers: 2,
      exclude: [
        ...configDefaults.exclude,
        "**/.claude/worktrees/**",
        "**/.praxis/worktrees/**",
        // node:test suite; executed by check:workflow, not Vitest.
      ],
    },
    ...(isMobile
      ? {
          base: "/m/",
          publicDir: "public-mobile",
          build: {
            outDir: "dist-mobile",
            emptyOutDir: true,
            rollupOptions: { input: "mobile.html" },
          },
        }
      : {}),

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 1421,
          }
        : undefined,
      watch: {
        // 3. tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});

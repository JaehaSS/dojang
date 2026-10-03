import { describe, expect, it } from "vitest";
import { directoryOnly, findFilePathSpans, isFilePathOnly } from "./file-path-links";

const found = (value: string) => findFilePathSpans(value).map((span) => span.text);

describe("findFilePathSpans", () => {
  it("finds a plain path in an agent answer", () => {
    expect(found("플랜 저장 완료: docs/plans/0014.2026-08-04-vlm-ocr-date-partition.md")).toEqual([
      "docs/plans/0014.2026-08-04-vlm-ocr-date-partition.md",
    ]);
  });

  it("keeps the line suffix with the match", () => {
    expect(found("resolveAgentLink는 src/App.tsx:1187 에 있다")).toEqual(["src/App.tsx:1187"]);
    expect(found("src-tauri/src/lib.rs:20:5 부터")).toEqual(["src-tauri/src/lib.rs:20:5"]);
  });

  it("stops at Korean particles glued to the extension", () => {
    expect(found("docs/STATE.md에서 확인한다")).toEqual(["docs/STATE.md"]);
  });

  it("finds every path in one paragraph", () => {
    expect(found("(src/lib/agent-link.ts, src/App.tsx) 두 곳")).toEqual([
      "src/lib/agent-link.ts",
      "src/App.tsx",
    ]);
  });

  it("takes tilde paths too — 에이전트가 홈 기준으로 적는 흔한 표기", () => {
    expect(found("AS-IS 정리 완료 — ~/work/docs/ai-as-is-architecture.md")).toEqual([
      "~/work/docs/ai-as-is-architecture.md",
    ]);
    expect(isFilePathOnly("~/work/docs/ai-as-is-architecture.md")).toBe(true);
  });

  it("takes absolute paths too", () => {
    expect(found("번들은 /Users/me/work/dist/index.js 로 나온다")).toEqual([
      "/Users/me/work/dist/index.js",
    ]);
  });

  it("finds paths whose segments are Korean — 볼트 문서의 흔한 이름", () => {
    expect(found("정리했다: 문서/학습자료/PaddleOCR-VL-SFT-아키텍처/평가-체계.md")).toEqual([
      "문서/학습자료/PaddleOCR-VL-SFT-아키텍처/평가-체계.md",
    ]);
    expect(found("/Users/me/Documents/문서/학습자료/평가-체계.md:12 참고")).toEqual([
      "/Users/me/Documents/문서/학습자료/평가-체계.md:12",
    ]);
    expect(isFilePathOnly("문서/학습자료/평가-체계.md")).toBe(true);
  });

  it("still stops at Korean particles after a Korean file name", () => {
    expect(found("문서/학습자료/평가-체계.md에서 본다")).toEqual(["문서/학습자료/평가-체계.md"]);
  });

  it("does not take a Korean 'or' slash as a path", () => {
    expect(found("학습/평가 체계를 나눈다")).toEqual([]);
    expect(found("개인/학습.자료 를 본다")).toEqual([]);
  });

  it("ignores slashes without an extension", () => {
    expect(found("and/or 를 Q4/2026 까지, docs/plans 아래에")).toEqual([]);
  });

  it("ignores dotted names without a slash", () => {
    expect(found("Next.js와 socket.io, package.json 그리고 1.2.3")).toEqual([]);
  });

  it("ignores paths that are part of a bigger token", () => {
    expect(found("https://example.com/docs/guide.md 를 보라")).toEqual([]);
    expect(found("git@github.com:acme/praxis.git 를 클론")).toEqual([]);
  });

  it("ignores numeric extensions", () => {
    expect(found("버전은 lib/v1.2 이고 tag/2026.08 이다")).toEqual([]);
  });
});

describe("isFilePathOnly", () => {
  it("accepts inline code that is nothing but a path", () => {
    expect(isFilePathOnly("docs/memory.md")).toBe(true);
    expect(isFilePathOnly(" src/App.tsx:1187 ")).toBe(true);
  });

  it("rejects inline code that merely contains a path", () => {
    expect(isFilePathOnly("npm run docs:project")).toBe(false);
    expect(isFilePathOnly("cat docs/memory.md")).toBe(false);
    expect(isFilePathOnly("grep -a 리터럴 src/lib.rs")).toBe(false);
  });

  it("accepts spaces in directory names of rooted paths only", () => {
    expect(isFilePathOnly("~/Documents/째하 지식창고/개인/README.md")).toBe(true);
    expect(isFilePathOnly("/Users/me/My Notes/a.md:3")).toBe(true);
    expect(isFilePathOnly("git add src/a.ts")).toBe(false);
    expect(isFilePathOnly("/usr/bin/env python3.11")).toBe(false);
    expect(isFilePathOnly("~/Documents/째하  지식창고/a.md")).toBe(false);
  });
});

describe("directoryOnly", () => {
  it("accepts inline code that is nothing but a directory", () => {
    expect(directoryOnly("docs/plans/")).toBe("docs/plans/");
    expect(directoryOnly(" ~/Documents/째하 지식창고/웹어셈블리/ ")).toBe(
      "~/Documents/째하 지식창고/웹어셈블리/",
    );
  });

  it("rejects files, bare slashes and commands", () => {
    expect(directoryOnly("docs/plans")).toBeNull();
    expect(directoryOnly("/")).toBeNull();
    expect(directoryOnly("ls docs/")).toBeNull();
  });
});

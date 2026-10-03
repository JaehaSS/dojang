import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import { unified } from "unified";
import { looksLikeHtml } from "./looks-like-html";

/**
 * 영어로 일하기 보조 — 입력창 ⌘J(한국어 초안 → 영어 프롬프트)와 답변 ⌥⌘J(영어 → 한국어).
 * 백엔드는 `src-tauri/src/translate.rs`. 브리프: docs/discovery/2026-09-23-english-prompt-assist-brief.md
 */

/** `reference`: 영어 제안을 입력창 위에 보여 주기만 한다. `replace`: 초안을 영어로 바꿔 넣는다. */
export type ComposerTranslateMode = "reference" | "replace";

export interface TranslateSettings {
  composer_mode: ComposerTranslateMode;
  model: string;
  /** ⌘J 결과와 답변 번역에서 공부할 표현을 짚는다(`src/lib/vocab.ts`). 예전에 저장된 설정엔 없으므로 없으면 켠 것으로 본다. */
  study_expressions?: boolean;
}

export type TranslateDirection = "ko_to_en_prompt" | "en_to_ko" | "en_to_ko_blocks" | "en_expressions";

export const DEFAULT_TRANSLATE_SETTINGS: TranslateSettings = { composer_mode: "reference", model: "sonnet", study_expressions: true };

export const studyExpressionsOn = (settings: TranslateSettings) => settings.study_expressions !== false;

export const translateText = (direction: TranslateDirection, text: string) =>
  invoke<string>("translate_text", { direction, text });

const HANGUL = /[ᄀ-ᇿ㄰-㆏가-힯]/g;
const LATIN = /[A-Za-z]/g;
const MAX_PROSE_TOKEN = 200;

/** 한글이 한 글자라도 있으면 영어로 다듬을 거리가 있다. */
export const hasHangul = (text: string): boolean => /[ᄀ-ᇿ㄰-㆏가-힯]/.test(text);

/**
 * 이미 한국어로 쓰인 답변인가 — 번역 버튼을 달지 않는다. 코드·경로가 섞인 한국어 답변도
 * 한국어로 보도록 글자 수 비율로 본다(라틴 글자는 영어 단어 기준 대략 음절의 3배).
 */
export function isMostlyKorean(text: string): boolean {
  // 코드·경로·URL은 어느 언어 답변에나 영문으로 섞이므로 세지 않는다.
  const prose = text
    .replace(/```[\s\S]*?(```|$)/g, " ")
    .replace(/`[^`]*`/g, " ")
    .split(/\s+/)
    // 경로·파일명 정규식은 긴 토큰에서 길이의 제곱으로 느려진다. 해시·base64처럼 긴 토큰은 문장이 아니므로 통째로 뺀다.
    .filter((token) => token.length <= MAX_PROSE_TOKEN)
    .map((token) =>
      token
        .replace(/https?:\/\/\S+/g, " ")
        // 경로(a/b)와 파일명(App.tsx). 문장 끝 마침표는 건드리지 않는다 — 점 뒤에 영숫자가 있어야 한다.
        .replace(/[\w.-]*[\w-][/\\][\w./\\-]*/g, " ")
        .replace(/[\w-]+\.[A-Za-z0-9]{1,6}\b/g, " "),
    )
    .join(" ");
  const hangul = prose.match(HANGUL)?.length ?? 0;
  const latin = prose.match(LATIN)?.length ?? 0;
  if (hangul === 0) return false;
  return hangul * 3 >= latin;
}

// ── 설정 스토어 — 입력창·대화창·설정 화면이 같은 값을 본다 ─────────────────────
let settings: TranslateSettings = DEFAULT_TRANSLATE_SETTINGS;
/** 저장된 값을 한 번이라도 읽었는가. 읽기 전에는 기본값이라, 그 위에 덮어 저장하면 저장된 값을 잃는다. */
let ready = false;
let loaded: Promise<void> | null = null;
let saving: Promise<unknown> = Promise.resolve();
const listeners = new Set<() => void>();
const RETRY_MS = 2000;

function publish(next: TranslateSettings): void {
  settings = next;
  listeners.forEach((listener) => listener());
}

function ensureLoaded(): Promise<void> {
  if (!loaded) {
    loaded = invoke<TranslateSettings>("translate_settings_get")
      .then((stored) => {
        ready = true;
        publish(stored);
      })
      .catch(() => {
        // 부팅 직후엔 DB가 아직 없을 수 있다. 이미 떠 있는 입력창은 다시 구독하지 않으므로 스스로 다시 읽는다.
        loaded = null;
        setTimeout(() => {
          if (listeners.size > 0) void ensureLoaded();
        }, RETRY_MS);
      });
  }
  return loaded;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  void ensureLoaded();
  return () => listeners.delete(listener);
}

export function useTranslateSettings(): TranslateSettings {
  return useSyncExternalStore(subscribe, () => settings);
}

/**
 * 바꾼 필드만 받는다. 저장은 차례로 — 연달아 바꿔도 앞의 변경 위에 합친다.
 * 저장된 값을 읽기 전이면 먼저 읽는다. 기본값 위에 합쳐 저장하면 다른 필드가 되돌아간다.
 */
export function saveTranslateSettings(patch: Partial<TranslateSettings>): Promise<void> {
  const run = saving.then(async () => {
    if (!ready) await ensureLoaded();
    if (!ready) throw new Error("저장된 설정을 읽지 못해 저장하지 않았습니다");
    const next = { ...settings, ...patch };
    await invoke("translate_settings_set", { settings: next });
    publish(next);
  });
  saving = run.catch(() => undefined);
  return run;
}

/** 테스트 전용 — 모듈 상태를 비운다. */
export function resetTranslateStateForTest(next: TranslateSettings = DEFAULT_TRANSLATE_SETTINGS): void {
  settings = next;
  ready = true;
  loaded = Promise.resolve();
  saving = Promise.resolve();
  replyCache.clear();
}

// ── 답변 문단 대역 — 영어 블록 바로 아래 그 블록의 번역을 둔다 ───────────────────
// 블록 경계는 모델이 아니라 파서가 정한다. 모델에게 번갈아 쓰게 하면 문단을 합치거나 빠뜨려
// 대응이 어긋나고, 원문까지 다시 쓰게 되어 원문이 변형될 수 있다.

/** 대역의 한 칸. `translation`이 null이면 코드 펜스처럼 번역하지 않고 원문만 보이는 블록이다. */
export interface BilingualBlock {
  source: string;
  translation: string | null;
}

/** 대역이 기본이다. 블록 대응이 어긋나거나 HTML 문서 답변이면 예전처럼 전체 번역을 원문 아래 붙인다. */
export type ReplyTranslation = { kind: "bilingual"; blocks: BilingualBlock[] } | { kind: "whole"; text: string };

/** 목록·표는 통째로 한 블록이다 — 항목마다 대역하면 답변 길이가 두 배가 된다. */
const TRANSLATABLE_BLOCKS = new Set(["paragraph", "heading", "list", "blockquote", "table"]);
const markdownParser = unified().use(remarkParse).use(remarkGfm);

/**
 * 최상위 블록 경계로 원문을 자른다. 조각을 이어 붙이면 원문과 같다 — 코드 블록은 번역 호출에
 * 들어가지도 않으므로 바이트 단위로 보존된다. 번역하지 않는 블록이 이어지면 한 조각으로 합친다.
 */
export function splitReplyBlocks(text: string): { source: string; translate: boolean }[] {
  const nodes = markdownParser.parse(text).children;
  const blocks: { source: string; translate: boolean }[] = [];
  nodes.forEach((node, i) => {
    const start = i === 0 ? 0 : node.position?.start.offset;
    const end = i === nodes.length - 1 ? text.length : nodes[i + 1].position?.start.offset;
    if (start === undefined || end === undefined) return;
    const source = text.slice(start, end);
    const translate = TRANSLATABLE_BLOCKS.has(node.type);
    const last = blocks[blocks.length - 1];
    if (!translate && last && !last.translate) last.source += source;
    else blocks.push({ source, translate });
  });
  return blocks;
}

/** 모델 응답이 요청한 개수의 문자열 배열인가. 코드 펜스로 감싸 오는 경우만 벗겨 준다. */
export function parseBlockTranslations(raw: string, count: number): string[] | null {
  const body = raw.trim().replace(/^```(?:json)?\s*\n([\s\S]*?)\n?```$/, "$1");
  try {
    const parsed: unknown = JSON.parse(body);
    if (!Array.isArray(parsed) || parsed.length !== count) return null;
    if (!parsed.every((item) => typeof item === "string")) return null;
    return parsed;
  } catch {
    return null;
  }
}

// 제목 번역을 제목으로 그리면 원문 제목과 같은 크기로 두 번 보인다 — 본문 글자로 낮춘다.
const unheading = (translation: string) => translation.replace(/^#{1,6}\s+/, "");

async function translateBilingual(text: string): Promise<ReplyTranslation> {
  const whole = async (): Promise<ReplyTranslation> => ({ kind: "whole", text: await translateText("en_to_ko", text) });
  // HTML 문서 답변은 Markdown이 통째로 샌드박스에 그린다 — 조각내면 문서가 깨진다.
  if (looksLikeHtml(text)) return whole();
  const blocks = splitReplyBlocks(text);
  const sources = blocks.filter((block) => block.translate).map((block) => block.source.trim());
  if (sources.length === 0) return whole();
  const translated = parseBlockTranslations(await translateText("en_to_ko_blocks", JSON.stringify(sources)), sources.length);
  if (!translated) return whole();
  let next = 0;
  return {
    kind: "bilingual",
    blocks: blocks.map((block) => ({
      source: block.source,
      translation: block.translate ? unheading(translated[next++]) : null,
    })),
  };
}

// ── 답변 번역 캐시 — 같은 메시지를 다시 펼치면 호출하지 않는다 ──────────────────
// 메모리에만 둔다. 실험 단계라 디스크·DB에 남기지 않는다.
const replyCache = new Map<string, Promise<ReplyTranslation>>();

export function translateReply(text: string): Promise<ReplyTranslation> {
  const hit = replyCache.get(text);
  if (hit) return hit;
  const pending = translateBilingual(text);
  replyCache.set(text, pending);
  // 실패는 캐시하지 않는다 — 다시 누르면 다시 시도한다.
  pending.catch(() => replyCache.delete(text));
  return pending;
}

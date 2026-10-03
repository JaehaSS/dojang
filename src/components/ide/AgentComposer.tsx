import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { HostId } from "../../lib/transport";
import { skillsList, pasteCaptureSave, type SkillMeta } from "../../lib/ipc";
import {
  matchMentionToken,
  filterMentionFiles,
  applyMention,
  handleMenuKey,
  filterSkills,
} from "../../lib/mention";
import {
  mergeMentionItems,
  mentionInsertText,
  type MentionItem,
} from "../../lib/mention-knowledge";
import { useKnowledgeMentions } from "../use-knowledge-mentions";
import { MentionDropdown } from "./MentionDropdown";
import { SkillDropdown } from "./SkillDropdown";
import { CaptureAttachmentChips } from "./CaptureAttachmentChips";
import { useSessionCaptureAttachments } from "./useSessionCaptureAttachments";
import { AgentComposerActions } from "./AgentComposerActions";
import { useTextareaUndo } from "./useTextareaUndo";
import { EnglishPromptAssistPanel, useEnglishPromptAssist } from "./useEnglishPromptAssist";
import { pushCapture } from "../../lib/designmode/store";
import { subscribeComposerFocus } from "../../lib/composer-focus";
import { findImageFile, blobToBase64 } from "../../lib/paste-image";
import { getTransport } from "../../lib/transport";

interface Props {
  value: string;
  onChange: (v: string) => void;
  /** 입력값과 캡처를 합친 최종 텍스트를 받는다. 입력창을 비우는 것은 호출자의 몫이다 —
   *  전송 가드(busy·빈 입력)에 걸렸을 때 사용자가 쓰던 내용을 잃지 않게. */
  onSend: (text: string, imagePaths: string[]) => void | boolean | Promise<void | boolean>;
  /** 인터럽트 (Ctrl-C) — 진행 중인 에이전트 턴 중단. */
  onInterrupt: () => void;
  /** ↑/↓ 입력 히스토리 탐색 (단일 행일 때만). */
  onHistory: (dir: -1 | 1) => void;
  /** worktree 상대 경로 목록 — @파일 멘션 자동완성. */
  files: string[];
  /** 현재 선택 레포 — /스킬 자동완성용. */
  repo?: string;
  /** Design Mode 캡처를 다른 세션과 격리하고 파일 삭제·이미지 전달에 사용하는 작업 id. */
  taskId: number;
  /** 작업이 사는 호스트 — 원격에는 붙여넣기를 저장할 로컬 워크트리가 없다. */
  host: HostId;
  /** 컨텍스트 잔량 게이지 — 소모 지점(전송) 곁에 둔다(설계 0044). 없으면 렌더하지 않는다. */
  gauge?: ReactNode;
  /** 중앙 diff가 보일 때 외부 포커스 요청을 받지 않는다. */
  active?: boolean;
  /** 초안이 사는 세션 좌표 — 세션이 바뀌면 되돌리기 스택을 버린다. */
  draftKey?: string | null;
  label?: string;
  sendLabel?: string;
  disabled?: boolean;
  attachments?: ReactNode;
}

/** 고도화된 에이전트 질의 입력창 — 멀티라인 + @파일 멘션 + /스킬 + 인터럽트 + 히스토리 +
 *  Design Mode 캡처 칩(D-1). Enter 전송 · Shift+Enter 줄바꿈 · @로 worktree 파일 멘션 · /로 스킬 슬래시 완성. */
export function AgentComposer({
  value,
  onChange,
  onSend,
  onInterrupt,
  onHistory,
  files,
  repo = "",
  taskId,
  host,
  gauge,
  active = true,
  draftKey = null,
  label,
  sendLabel,
  disabled = false,
  attachments,
}: Props) {
  const taRef = useRef<HTMLTextAreaElement>(null);
  const undo = useTextareaUndo({ value, setValue: onChange, ref: taRef, resetKey: draftKey });
  const assist = useEnglishPromptAssist({ value, onChange, draftKey });
  const clearAssist = assist.clear;
  // 보냈으면 제안·원문 패널을 걷는다. 전송 가드에 걸려(false) 초안이 남았으면 그대로 둔다.
  const sendAndClearAssist = useCallback(async (text: string, imagePaths: string[]) => {
    const result = await onSend(text, imagePaths);
    if (result !== false) clearAssist();
    return result;
  }, [onSend, clearAssist]);
  const captureAttachments = useSessionCaptureAttachments({ taskId, host, value, onSend: sendAndClearAssist });
  const [pasteError, setPasteError] = useState<string | null>(null);

  // @파일 멘션 드롭다운
  const [menuOpen, setMenuOpen] = useState(false);
  /** 파일 경로만 — 지식 결과는 별도로 와서 렌더 직전에 합쳐진다. */
  const [fileNames, setFileNames] = useState<string[]>([]);
  const [sel, setSel] = useState(0);
  /** 활성 @토큰. null이면 멘션 중이 아니다. */
  const [token, setToken] = useState<string | null>(null);

  // /스킬 드롭다운
  const [skillOpen, setSkillOpen] = useState(false);
  const [skills, setSkills] = useState<SkillMeta[]>([]);
  const [skillSel, setSkillSel] = useState(0);
  const skillCacheRef = useRef<{ host: HostId; repo: string; list: SkillMeta[] } | null>(null);

  // ⌘L로 에디터 선택을 첨부한 직후 바로 질문을 타이핑할 수 있도록 입력창을 잡아준다.
  useEffect(
    () => subscribeComposerFocus(taskId, () => active && taRef.current?.focus()),
    [taskId, active],
  );

  // 내용에 맞춰 높이 자동 조절 (최대 ~10행).
  useLayoutEffect(() => {
    const ta = taRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, 208)}px`;
  }, [value]);

  // 캐럿 앞의 @토큰 감지 → 멘션 메뉴.
  useEffect(() => {
    const ta = taRef.current;
    const caret = ta?.selectionStart ?? value.length;
    const tk = matchMentionToken(value.slice(0, caret));
    setToken(tk);
    if (tk === null) {
      setFileNames([]);
      return;
    }
    setFileNames(filterMentionFiles(files, tk));
    setSel(0);
  }, [value, files]);

  // 외부 지식 — 파일 결과를 기다리지 않고 따로 온다 (설계 0020 DR-7).
  const { hits: knowledgeHits, pending: knowledgePending } = useKnowledgeMentions(token);
  const mentionItems = useMemo(
    () => mergeMentionItems(fileNames, knowledgeHits),
    [fileNames, knowledgeHits],
  );

  // Esc로 닫은 뒤 지식 결과가 늦게 도착해도 메뉴가 되살아나지 않게, 항목이 바뀔 때만 갱신한다.
  useEffect(() => {
    setMenuOpen(token !== null && mentionItems.length > 0);
  }, [token, mentionItems]);

  // /로 시작하고 공백 없으면 → 스킬 드롭다운.
  useEffect(() => {
    const ta = taRef.current;
    const caret = ta?.selectionStart ?? value.length;
    const before = value.slice(0, caret);
    const m = before.match(/^\/([^\s]*)$/);
    if (!m) {
      setSkillOpen(false);
      return;
    }
    const prefix = m[1].toLowerCase();

    const applyFilter = (list: SkillMeta[]) => {
      const filtered = filterSkills(list, prefix);
      setSkills(filtered);
      setSkillSel(0);
      setSkillOpen(filtered.length > 0);
    };

    // 캐시 히트
    const cached = skillCacheRef.current;
    if (cached && cached.host === host && cached.repo === repo) {
      applyFilter(cached.list);
      return;
    }
    // lazy fetch — 목록은 작업이 사는 호스트에서 온다.
    skillsList(host, repo).then((list) => {
      skillCacheRef.current = { host, repo, list };
      applyFilter(list);
    });
  }, [value, host, repo]);

  // host·repo 변경 시 캐시 무효화.
  useEffect(() => {
    const cached = skillCacheRef.current;
    if (cached && (cached.host !== host || cached.repo !== repo)) {
      skillCacheRef.current = null;
      setSkillOpen(false);
    }
  }, [host, repo]);

  const insertMention = (item: MentionItem) => {
    const ta = taRef.current;
    const caret = ta?.selectionStart ?? value.length;
    // 파일은 경로 그대로(기존 계약), 지식은 출처가 보이는 라벨로 들어간다.
    const { value: next, caret: pos } = applyMention(value, caret, mentionInsertText(item));
    onChange(next);
    setMenuOpen(false);
    requestAnimationFrame(() => {
      taRef.current?.setSelectionRange(pos, pos);
      taRef.current?.focus();
    });
  };

  const insertSkill = (name: string) => {
    const ta = taRef.current;
    const caret = ta?.selectionStart ?? value.length;
    const before = value.slice(0, caret).replace(/^\/[^\s]*$/, `/${name} `);
    const next = before + value.slice(caret);
    onChange(next);
    setSkillOpen(false);
    requestAnimationFrame(() => {
      taRef.current?.setSelectionRange(before.length, before.length);
      taRef.current?.focus();
    });
  };

  // 클립보드 이미지 붙여넣기 → 백엔드가 캡처 레코드로 저장 → 칩으로 첨부. 이후는 기존
  // 캡처 파이프라인(전송 시 프롬프트 주입 + 지원 벤더 --image 전달 + 종결 정리)을 그대로 탄다.
  // 원격(Runner) 작업은 로컬 worktree가 없어 저장할 곳이 없다 — 붙여넣기를 가로채지 않는다.
  const onPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const file = findImageFile(e.clipboardData);
    if (!file || getTransport(host).kind === "remote") return;
    e.preventDefault();
    void (async () => {
      try {
        const record = await pasteCaptureSave(taskId, await blobToBase64(file), file.type);
        pushCapture(taskId, record);
        setPasteError(null);
      } catch (error) {
        setPasteError(String(error));
      }
    })();
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (undo.onKeyDown(e)) return;
    // ⌘J 영어로 다듬기 — 조합 중이면 스스로 무시한다.
    if (assist.onKeyDown(e)) return;
    // Korean/Japanese composition confirmation must not submit (including Safari keyCode 229).
    if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
    // /스킬 드롭다운 우선 (↑↓/Tab/Esc — Enter는 전송으로 fallthrough).
    if (
      skillOpen &&
      handleMenuKey(e, {
        items: skills,
        sel: skillSel,
        setSel: setSkillSel,
        onSelect: (sk) => insertSkill(sk.name),
        close: () => setSkillOpen(false),
      })
    )
      return;
    // @파일 멘션 드롭다운 (↑↓/Enter/Tab/Esc).
    if (
      menuOpen &&
      handleMenuKey(e, { items: mentionItems, sel, setSel, onSelect: insertMention, close: () => setMenuOpen(false), enterSelects: true })
    )
      return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (disabled || !value.trim()) return;
      void captureAttachments.send();
      return;
    }
    // 히스토리: 단일 행일 때만 ↑↓ (멀티라인 편집을 방해하지 않게).
    if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !value.includes("\n")) {
      e.preventDefault();
      onHistory(e.key === "ArrowUp" ? -1 : 1);
    }
  };

  return (
    <div className="relative">
      {label && <label htmlFor={`agent-composer-${draftKey ?? taskId}`} className="mb-1 block text-xs text-text-secondary">{label}</label>}
      {attachments}
      <SkillDropdown open={skillOpen} items={skills} sel={skillSel} onHover={setSkillSel} onSelect={insertSkill} />
      <MentionDropdown
        open={menuOpen}
        items={mentionItems}
        sel={sel}
        token={token ?? ""}
        knowledgePending={knowledgePending}
        onHover={setSel}
        onSelect={insertMention}
      />
      <CaptureAttachmentChips
        captures={captureAttachments.captures}
        onRemove={captureAttachments.remove}
      />
      <EnglishPromptAssistPanel state={assist.state} onDismiss={assist.clear} />
      {pasteError && (
        <div className="mb-1 text-xs text-status-failed">이미지 첨부 실패: {pasteError}</div>
      )}
      <div className="flex items-end gap-2 border border-border-strong rounded-md px-3 py-2 focus-within:border-primary bg-bg">
        <textarea
          id={`agent-composer-${draftKey ?? taskId}`}
          aria-label={label ?? "에이전트에게 질의"}
          ref={taRef}
          rows={1}
          className="flex-1 bg-transparent outline-none text-md text-text placeholder:text-text-muted resize-none leading-relaxed max-h-52"
          placeholder="에이전트에게 질의 — Enter 전송 · Shift+Enter 줄바꿈 · @파일 · /스킬 · ⌘L 선택 코드 · 이미지 붙여넣기 · ↑↓ 히스토리 · ⌘J 영어로"
          value={value}
          onChange={undo.onChange}
          onKeyDown={onKeyDown}
          onSelect={undo.onSelect}
          onCompositionStart={undo.onCompositionStart}
          onCompositionEnd={undo.onCompositionEnd}
          onPaste={onPaste}
        />
        {gauge}
        <AgentComposerActions
          canSend={Boolean(value.trim()) && !disabled}
          sendLabel={sendLabel}
          onInterrupt={onInterrupt}
          onSend={() => { if (!disabled) void captureAttachments.send(); }}
        />
      </div>
    </div>
  );
}

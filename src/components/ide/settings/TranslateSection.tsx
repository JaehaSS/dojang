import { useState } from "react";
import { saveTranslateSettings, studyExpressionsOn, useTranslateSettings, type ComposerTranslateMode } from "../../../lib/translate";
import { SettingRow, SettingSection, Switch } from "./SettingRow";

/** 실측(짧은 한 문장, CLI 2.1.280): sonnet+low 중앙값 2.7초, haiku+low 4.7초. 이름만 보고 고르지 않게 적어 둔다. */
const MODELS = [
  { id: "sonnet", label: "sonnet — 기본, 실측 약 2.7초" },
  { id: "haiku", label: "haiku — 실측 약 4.7초" },
];

const selectCls = "bg-bg border border-border rounded px-2 py-1 text-xs text-text";

/** 영어로 일하기 보조 — 입력창 ⌘J와 답변 번역(⌥⌘J)이 쓰는 설정. */
export function TranslateSection() {
  const settings = useTranslateSettings();
  const [error, setError] = useState<string | null>(null);
  const save = (patch: Partial<typeof settings>) => {
    saveTranslateSettings(patch).then(
      () => setError(null),
      (e: unknown) => setError(String(e)),
    );
  };
  const models = MODELS.some((m) => m.id === settings.model)
    ? MODELS
    : [...MODELS, { id: settings.model, label: settings.model }];

  return (
    <SettingSection
      id="english-assist"
      title="영어로 일하기 보조"
      risk="cost"
      hint="입력창 ⌘J로 한국어 초안을 영어 프롬프트로, 답변의 번역 버튼·⌥⌘J로 영어 답변을 한국어로 봅니다. 누를 때마다 claude를 도구 없이 한 번 호출합니다."
    >
      <SettingRow title="⌘J 결과" hint="참고로 보기: 영어 제안을 입력창 위에 띄우고 직접 입력합니다. 바꿔 넣기: 초안을 영어로 바꿉니다(⌘Z로 되돌림). 어느 쪽도 자동으로 보내지 않습니다.">
        <select
          aria-label="⌘J 결과"
          className={selectCls}
          value={settings.composer_mode}
          onChange={(e) => save({ composer_mode: e.target.value as ComposerTranslateMode })}
        >
          <option value="reference">참고로 보기</option>
          <option value="replace">바꿔 넣기</option>
        </select>
      </SettingRow>
      <SettingRow
        title="공부할 표현 짚기"
        hint="⌘J 영어 제안과 답변 번역에서 표현 3~5개에 밑줄을 긋습니다. 올리면 뜻이 보이고 단어장에 저장할 수 있습니다. 켜 두면 결과마다 호출이 하나 더 듭니다."
      >
        <Switch on={studyExpressionsOn(settings)} onClick={() => save({ study_expressions: !studyExpressionsOn(settings) })} />
      </SettingRow>
      <SettingRow title="모델" hint="effort는 low로 고정합니다.">
        <select aria-label="번역 모델" className={selectCls} value={settings.model} onChange={(e) => save({ model: e.target.value })}>
          {models.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </SettingRow>
      {error && <div className="text-xs text-status-failed">저장하지 못했습니다: {error}</div>}
    </SettingSection>
  );
}

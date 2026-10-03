import { useEffect, useState } from "react";
import {
  notificationPermission,
  notificationSettingsSet,
  notificationSnapshot,
  notificationTest,
} from "../../../lib/notifications";
import { VoiceSettingsCard } from "../VoiceSettingsCard";
import { SettingRow, SettingSection, SettingsTabShell, Switch, TabSummary } from "./SettingRow";

/** 알림·음성 — 앱이 사용자를 부르는 경로와, 사용자가 손을 안 쓰고 앱을 부르는 경로. */
export function NotificationsTab() {
  const [enabled, setEnabled] = useState(false);
  const [permission, setPermission] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    notificationSnapshot().then((snapshot) => setEnabled(snapshot.enabled)).catch(() => {});
    notificationPermission().then(setPermission).catch((reason) => setError(String(reason)));
  }, []);

  const toggleNotifications = async () => {
    const next = !enabled;
    setEnabled(next);
    try {
      setEnabled((await notificationSettingsSet(next)).enabled);
      setError(null);
    } catch (reason) {
      setEnabled(!next);
      setError(String(reason));
    }
  };

  const testNotification = async () => {
    try {
      await notificationTest();
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  return (
    <SettingsTabShell>
      <TabSummary
        items={[`OS 알림 ${enabled ? "켜짐" : "꺼짐"}`, permission ? `권한 ${permission}` : null]}
      />

      <SettingSection
        id="os-notifications"
        title="OS 알림"
        hint="에이전트가 질문하거나 검토 대기에 들어가면 시스템 알림으로 부릅니다."
      >
        <SettingRow
          title="알림 보내기"
          hint={
            <>
              {permission === "unknown"
                ? "이 앱은 OS 권한 상태를 확인할 수 없습니다. 시스템 알림 설정에서 Dojang을 확인하세요."
                : `권한 상태: ${permission ?? "확인 중"}`}
              {error && <div className="text-xs text-status-failed">{error}</div>}
              <button
                onClick={() => void testNotification()}
                className="mt-1 block text-xs text-text-secondary hover:text-text"
              >
                테스트 알림 보내기
              </button>
            </>
          }
        >
          <Switch on={enabled} onClick={() => void toggleNotifications()} label="OS 알림" />
        </SettingRow>
      </SettingSection>

      <SettingSection
        id="voice"
        title="음성 입력"
        hint={
          <>
            핫키를 <b>누르고 있는 동안</b> 녹음하고 놓으면 전사한다. 커맨드는 화면 전환·전송,
            받아쓰기는 프롬프트 입력이며 <b>받아쓰기는 자동 전송하지 않는다</b>. 별도 STT 서버가
            필요하다 — OpenAI 호환 전사 API면 무엇이든 꽂힌다.
          </>
        }
      >
        <VoiceSettingsCard />
      </SettingSection>
    </SettingsTabShell>
  );
}

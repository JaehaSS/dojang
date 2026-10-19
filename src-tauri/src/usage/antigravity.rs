//! Antigravity의 그룹별 quota 스냅샷. 로컬 앱 조회 실패 시 Keychain OAuth 조회로 이어간다.
//! 앱을 실행하지 않는다. CSRF와 OAuth 자격증명은 메모리에서만 쓴다.

use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use super::{QuotaGroup, UsageWindow, VendorUsage};

const CACHE_SECS: i64 = 60;
const RPC: &str = "exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary";
static CACHE: Mutex<Option<(i64, i64, VendorUsage)>> = Mutex::const_new(None);

fn blank(status: &str, detail: &str) -> VendorUsage {
    VendorUsage::blank("agy", "Antigravity", status, Some(detail.into()))
}

pub(super) fn parse_summary(value: &Value, now: i64) -> Option<VendorUsage> {
    // 로컬 Connect RPC는 response로 감싸고 Google API는 groups를 최상위에 준다.
    let groups = value
        .get("response")
        .unwrap_or(value)
        .get("groups")?
        .as_array()?;
    let mut quota_groups = Vec::new();
    let mut gemini = None;
    for group in groups {
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        let mut five_hour = None;
        let mut weekly = None;
        let mut is_gemini = false;
        for bucket in buckets {
            let Some(id) = bucket.get("bucketId").and_then(Value::as_str) else {
                continue;
            };
            let slot = match bucket.get("window").and_then(Value::as_str) {
                Some("5h") => &mut five_hour,
                Some("weekly") => &mut weekly,
                _ => continue,
            };
            // protobuf JSON은 기본값 0을 생략할 수 있다. 유효한 bucket에서만 생략을 0으로 읽는다.
            let fraction = match bucket.get("remainingFraction") {
                None => 0.0,
                Some(value) => match value.as_f64() {
                    Some(number) if number.is_finite() && (0.0..=1.0).contains(&number) => number,
                    _ => continue,
                },
            };
            let minutes = if bucket["window"] == "5h" {
                300
            } else {
                10_080
            };
            let window = UsageWindow {
                used_percent: (1.0 - fraction) * 100.0,
                resets_at: bucket
                    .get("resetTime")
                    .and_then(Value::as_str)
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.timestamp()),
                window_minutes: Some(minutes),
            };
            // 중복 bucket이 오면 더 소진된 관측을 유지한다. 그룹 간에는 합치지 않는다.
            if slot
                .as_ref()
                .map(|old: &UsageWindow| old.used_percent < window.used_percent)
                .unwrap_or(true)
            {
                *slot = Some(window);
            }
            is_gemini |= id.starts_with("gemini-");
        }
        if five_hour.is_none() && weekly.is_none() {
            continue;
        }
        if is_gemini {
            gemini = Some((five_hour.clone(), weekly.clone()));
        }
        quota_groups.push(QuotaGroup {
            label: group
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or("모델 그룹")
                .into(),
            five_hour,
            weekly,
        });
    }
    if quota_groups.is_empty() {
        return None;
    }
    // 기존 단일 창 소비자에게는 Dojang의 agy 프리셋이 쓰는 Gemini 한도만 제공한다.
    let (five_hour, weekly) = gemini.unwrap_or_default();
    Some(VendorUsage {
        vendor: "agy".into(),
        label: "Antigravity".into(),
        status: "ok".into(),
        detail: None,
        plan: None,
        five_hour,
        weekly,
        quota_groups,
        source: Some("antigravity-local".into()),
        updated_at: Some(now),
    })
}

// Debug를 구현하지 않는다: args에서 얻는 CSRF가 로그로 노출되지 않게 한다.
struct Server {
    pid: u32,
    csrf: String,
    hub: bool,
}

fn parse_process(line: &str) -> Option<Server> {
    let line = line.trim();
    let (pid, args) = line.split_once(char::is_whitespace)?;
    let args = args.trim_start();
    let marker = args.find(" --")?;
    let executable = &args[..marker];
    let name = std::path::Path::new(executable).file_name()?.to_str()?;
    // 다른 프로세스의 인수에 들어 있는 명령 문자열과 다른 앱의 language server를 제외한다.
    if !matches!(
        name,
        "language_server" | "language_server_macos_arm" | "language_server_macos_x64"
    ) || !executable.starts_with("/Applications/Antigravity")
        || !executable.contains(".app/Contents/Resources/")
    {
        return None;
    }
    let flags: Vec<_> = args[marker..].split_whitespace().collect();
    let csrf = flags.iter().enumerate().find_map(|(index, flag)| {
        flag.strip_prefix("--csrf_token=").or_else(|| {
            (*flag == "--csrf_token")
                .then(|| flags.get(index + 1).copied())
                .flatten()
        })
    })?;
    if csrf.is_empty() || csrf.starts_with("--") {
        return None;
    }
    Some(Server {
        pid: pid.parse().ok()?,
        csrf: csrf.into(),
        hub: executable.contains("/Resources/bin/"),
    })
}

fn parse_ports(output: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for line in output.lines() {
        let Some(address) = line.strip_prefix('n') else {
            continue;
        };
        let Some((host, port)) = address.rsplit_once(':') else {
            continue;
        };
        if !matches!(host, "127.0.0.1" | "[::1]" | "*" | "localhost") {
            continue;
        }
        if let Ok(port) = port.parse::<u16>() {
            if port > 0 && !ports.contains(&port) {
                ports.push(port);
            }
        }
    }
    // Hub에는 브라우저용 proxy listener도 많다. 앞쪽 LS listener만 조회한다.
    ports.truncate(4);
    ports
}

#[cfg(target_os = "macos")]
async fn command(program: &str, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
async fn fetch(now: i64) -> VendorUsage {
    let Some(processes) = command("/bin/ps", &["-axo", "pid=,args="]).await else {
        return blank("error", "Antigravity의 실행 상태를 확인하지 못했습니다");
    };
    let mut servers: Vec<_> = processes.lines().filter_map(parse_process).collect();
    servers.sort_by_key(|server| !server.hub);
    if servers.is_empty() {
        return blank(
            "no_data",
            "Antigravity Hub 또는 IDE를 실행하면 사용량을 조회할 수 있습니다",
        );
    }
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return blank("error", "로컬 사용량 조회를 준비하지 못했습니다");
    };
    for server in servers.into_iter().take(4) {
        let pid = server.pid.to_string();
        let Some(listeners) = command(
            "/usr/sbin/lsof",
            &["-nP", "-a", "-p", &pid, "-iTCP", "-sTCP:LISTEN", "-Fpn"],
        )
        .await
        else {
            continue;
        };
        for port in parse_ports(&listeners) {
            // Hub/IDE가 제공하는 HTTP listener를 사용한다. 로컬 TLS 인증서 검증을 끄지 않는다.
            let Ok(response) = client
                .post(format!("http://127.0.0.1:{port}/{RPC}"))
                .header("X-Codeium-Csrf-Token", &server.csrf)
                .header("Connect-Protocol-Version", "1")
                .json(&serde_json::json!({}))
                .send()
                .await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let Ok(value) = response.json::<Value>().await else {
                continue;
            };
            if let Some(usage) = parse_summary(&value, now) {
                return usage;
            }
        }
    }
    blank(
        "error",
        "Antigravity 앱의 사용량 응답을 받지 못했습니다. 앱의 로그인 상태를 확인하세요",
    )
}

#[cfg(not(target_os = "macos"))]
async fn fetch(_now: i64) -> VendorUsage {
    blank(
        "unsupported",
        "이 플랫폼의 Antigravity 로컬 사용량 조회는 아직 지원하지 않습니다",
    )
}

fn retain_last(fresh: VendorUsage, previous: Option<&VendorUsage>) -> VendorUsage {
    if fresh.status == "ok" || fresh.status == "unauthenticated" {
        return fresh;
    }
    if let Some(previous) = previous.filter(|value| !value.quota_groups.is_empty()) {
        let mut stale = previous.clone();
        stale.status = "stale".into();
        stale.detail = fresh.detail;
        return stale;
    }
    fresh
}

pub(super) async fn usage(now: i64, force: bool) -> VendorUsage {
    // 조회를 직렬화해 홈/에디터의 동시 폴링이 같은 API를 중복 호출하지 않게 한다.
    let mut cache = CACHE.lock().await;
    if !force {
        if let Some((checked, interval, usage)) = cache.as_ref() {
            if now - checked < *interval {
                return usage.clone();
            }
        }
    }
    let local = tokio::time::timeout(Duration::from_secs(8), fetch(now))
        .await
        .unwrap_or_else(|_| blank("error", "Antigravity 사용량 조회 시간이 초과됐습니다"));
    let fresh = if local.status == "ok" {
        local
    } else {
        tokio::time::timeout(
            Duration::from_secs(20),
            super::antigravity_oauth::fetch(now),
        )
        .await
        .unwrap_or_else(|_| blank("error", "Antigravity 사용량 API 조회 시간이 초과됐습니다"))
    };
    // stale 값의 출처와 재조회 간격은 다르다. 로컬 마지막 값이어도 remote 실패는 5분 후 재시도한다.
    let interval = if fresh.source.as_deref() == Some("antigravity-local") {
        CACHE_SECS
    } else {
        300
    };
    let usage = retain_last(fresh, cache.as_ref().map(|(_, _, usage)| usage));
    *cache = Some((now, interval, usage.clone()));
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({"response":{"groups":[
            {"displayName":"Gemini Models","buckets":[
                {"bucketId":"gemini-weekly","window":"weekly","remainingFraction":0.98,"resetTime":"2030-10-13T06:25:12Z"},
                {"bucketId":"gemini-5h","window":"5h","remainingFraction":0.9,"resetTime":"2030-10-08T00:56:09Z"}]},
            {"displayName":"Claude and GPT models","buckets":[
                {"bucketId":"3p-5h","window":"5h","remainingFraction":0.25}]}]}})
    }

    #[test]
    fn maps_groups_without_merging_and_preserves_weekly_reset() {
        let usage = parse_summary(&fixture(), 1000).unwrap();
        assert_eq!(usage.quota_groups.len(), 2);
        assert!((usage.five_hour.unwrap().used_percent - 10.0).abs() < 0.001);
        let weekly = usage.weekly.unwrap();
        assert!((weekly.used_percent - 2.0).abs() < 0.001);
        assert_eq!(
            weekly.resets_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2030-10-13T06:25:12Z")
                    .unwrap()
                    .timestamp()
            )
        );
        assert_eq!(weekly.window_minutes, Some(10_080));
        assert_eq!(
            usage.quota_groups[1]
                .five_hour
                .as_ref()
                .unwrap()
                .used_percent,
            75.0
        );
        assert!(usage.quota_groups[1].weekly.is_none());
        assert_eq!(usage.updated_at, Some(1000));
    }

    #[test]
    fn remote_summary_has_the_same_groups_without_connect_envelope() {
        let value = fixture();
        let usage = parse_summary(&value["response"], 1000).unwrap();
        assert_eq!(usage.quota_groups.len(), 2);
        assert!(usage.five_hour.is_some());
        assert!(usage.weekly.is_some());
    }

    #[test]
    fn zero_quota_including_protobuf_default_is_exhausted() {
        let mut value = fixture();
        value["response"]["groups"][0]["buckets"][1]
            .as_object_mut()
            .unwrap()
            .remove("remainingFraction");
        assert_eq!(
            parse_summary(&value, 1000)
                .unwrap()
                .five_hour
                .unwrap()
                .used_percent,
            100.0
        );
        value["response"]["groups"][0]["buckets"][1]["remainingFraction"] = json!(0);
        assert_eq!(
            parse_summary(&value, 1000)
                .unwrap()
                .five_hour
                .unwrap()
                .used_percent,
            100.0
        );
    }

    #[test]
    fn malformed_and_unknown_buckets_never_invent_quota() {
        assert!(parse_summary(&json!({}), 0).is_none());
        assert!(parse_summary(&json!({"response":{"groups":[]}}), 0).is_none());
        for fraction in [json!(-1), json!(2), json!("0.5"), Value::Null] {
            let value = json!({"response":{"groups":[{"buckets":[{"bucketId":"gemini-5h","window":"5h","remainingFraction":fraction}]}]}});
            assert!(parse_summary(&value, 0).is_none());
        }
        assert!(parse_summary(&json!({"response":{"groups":[{"buckets":[{"window":"5h"},{"bucketId":"new","window":"monthly"}]}]}}), 0).is_none());
    }

    #[test]
    fn discovers_only_real_antigravity_servers_with_spaces_in_path() {
        let server = parse_process(" 123 /Applications/Antigravity IDE.app/Contents/Resources/app/extensions/antigravity/bin/language_server_macos_arm --csrf_token dummy --subclient_type ide").unwrap();
        assert_eq!(server.pid, 123);
        assert_eq!(server.csrf, "dummy");
        assert!(!server.hub);
        assert!(parse_process("12 /Applications/Antigravity.app/Contents/Resources/bin/language_server --csrf_token=dummy").unwrap().hub);
        assert!(parse_process("12 /bin/zsh -c /Applications/Antigravity.app/Contents/Resources/bin/language_server --csrf_token dummy").is_none());
        assert!(parse_process(
            "12 /Applications/Other.app/Contents/Resources/bin/language_server --csrf_token dummy"
        )
        .is_none());
        assert!(parse_process("12 /Applications/Antigravity.app/Contents/Resources/bin/language_server --csrf_token --standalone").is_none());
    }

    #[test]
    fn listener_discovery_ignores_remote_hosts_duplicates_and_bad_ports() {
        assert_eq!(parse_ports("p123\nn127.0.0.1:5000\nn*:5001\nn[::1]:5002\nn10.0.0.2:80\nn127.0.0.1:5000\nn*:0\nn*:70000"), vec![5000, 5001, 5002]);
    }

    #[test]
    fn failure_preserves_values_and_original_observation_time() {
        let previous = parse_summary(&fixture(), 1000).unwrap();
        let stale = retain_last(blank("no_data", "앱 종료"), Some(&previous));
        assert_eq!(stale.status, "stale");
        assert_eq!(stale.updated_at, Some(1000));
        assert_eq!(stale.quota_groups.len(), 2);
        assert_eq!(
            retain_last(blank("no_data", "앱 종료"), None).status,
            "no_data"
        );
        let signed_out = retain_last(blank("unauthenticated", "로그인 만료"), Some(&previous));
        assert_eq!(signed_out.status, "unauthenticated");
        assert!(signed_out.quota_groups.is_empty());
    }

    /// 명시적으로 실행하는 읽기 전용 실환경 검증. 사용자 이름·토큰·응답 원문은 출력하지 않는다.
    #[tokio::test]
    #[ignore = "requires a running Antigravity Hub or IDE"]
    async fn live_local_quota() {
        let usage = fetch(super::super::now_secs()).await;
        assert_eq!(usage.status, "ok", "{:?}", usage.detail);
        assert!(!usage.quota_groups.is_empty());
        assert!(usage.five_hour.is_some());
        assert!(usage.weekly.is_some());
        println!(
            "Antigravity local quota: {} groups, 5h and weekly present",
            usage.quota_groups.len()
        );
    }
}

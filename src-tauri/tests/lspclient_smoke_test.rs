//! 실제 언어 서버와의 왕복 스모크 — 프레이밍·핸드셰이크·좌표 변환이 한 줄로 이어지는지 본다.
//!
//! 서버 설치에 의존하므로 `#[ignore]`다. 유닛 테스트는 프로토콜 조각만 검증하니,
//! 붙는지는 여기서 확인한다:
//!   cargo test --test lspclient_smoke_test -- --ignored --nocapture

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use praxis_lib::lspclient::{GotoKind, LspPool};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri의 부모가 레포 루트")
        .to_path_buf()
}

/// 파일에서 `needle`이 처음 나오는 (line, column)을 Monaco 좌표(1-based)로 찾는다.
/// 줄 번호를 테스트에 박아두면 대상 파일이 한 줄만 밀려도 깨지므로 런타임에 계산한다.
fn locate(text: &str, line_contains: &str, needle: &str) -> (u32, u32) {
    let (index, line) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains(line_contains))
        .unwrap_or_else(|| panic!("'{line_contains}' 를 담은 줄을 찾지 못했습니다"));
    let column = line.find(needle).expect("needle이 그 줄에 있어야 한다");
    (index as u32 + 1, column as u32 + 1)
}

#[tokio::test]
#[ignore = "typescript-language-server 설치 필요"]
async fn typescript_definition_lands_on_the_declaration() {
    let root = repo_root();
    let rel = "src/lib/lsp.ts";
    let text = std::fs::read_to_string(root.join(rel)).expect("대상 파일 읽기");

    // resolveOutcome 본문의 dedupeTargets 호출부 → 같은 파일의 dedupeTargets 선언으로 가야 한다.
    let (line, column) = locate(&text, "const unique = dedupeTargets(", "dedupeTargets");
    let pool = LspPool::default();

    let targets = pool
        .goto(1, &root, rel, &text, line, column, GotoKind::Definition)
        .await
        .expect("정의 조회 성공");

    let expected_line = locate(&text, "export function dedupeTargets", "dedupeTargets").0;
    println!("{line}:{column} → {targets:?} (기대 줄 {expected_line})");
    assert!(
        targets
            .iter()
            .any(|t| t.path.as_deref() == Some(rel) && t.line == expected_line),
        "선언 줄({expected_line})을 가리켜야 하는데 {targets:?}",
    );
}

#[tokio::test]
#[ignore = "typescript-language-server 설치 필요"]
async fn unsaved_edit_is_visible_to_the_server() {
    let root = repo_root();
    let rel = "src/lib/lsp.ts";
    let disk = std::fs::read_to_string(root.join(rel)).expect("대상 파일 읽기");

    // 디스크에 없는 함수를 버퍼에만 추가하고, 그 함수의 호출부에서 정의를 묻는다.
    // 서버가 저장본만 본다면 여기서 결과가 비어 실패한다.
    let edited = format!("{disk}\nfunction 임시함수() {{ return 1; }}\nconst 사용 = 임시함수();\n");
    let (line, column) = locate(&edited, "const 사용 = 임시함수()", "임시함수()");
    let expected_line = locate(&edited, "function 임시함수()", "임시함수").0;

    let pool = LspPool::default();
    let targets = pool
        .goto(2, &root, rel, &edited, line, column, GotoKind::Definition)
        .await
        .expect("정의 조회 성공");

    println!("미저장 편집 {line}:{column} → {targets:?}");
    assert!(
        targets.iter().any(|t| t.line == expected_line),
        "버퍼에만 있는 선언({expected_line})을 찾아야 하는데 {targets:?}",
    );
}

#[tokio::test]
#[ignore = "rust-analyzer 설치 필요 (rustup component add rust-analyzer). 첫 인덱싱이 느리면 타임아웃될 수 있다"]
async fn rust_definition_lands_on_the_declaration() {
    let root = repo_root();
    let rel = "src/lspclient/protocol.rs";
    let text = std::fs::read_to_string(root.join("src-tauri").join(rel)).expect("대상 파일 읽기");
    let (line, column) = locate(&text, "let mut out = format!(\"Content-Length", "format!");

    let pool = LspPool::default();
    // 워크트리 루트는 Cargo.toml이 있는 src-tauri다.
    let result = pool
        .goto(
            4,
            &root.join("src-tauri"),
            rel,
            &text,
            line,
            column,
            GotoKind::Definition,
        )
        .await;

    // 서버가 없거나 인덱싱이 안 끝났으면 사유가 그대로 보여야 한다 — 빈손 실패는 진단이 안 된다.
    match result {
        Ok(targets) => println!("rust 정의 → {targets:?}"),
        Err(reason) => panic!("정의 조회 실패(사유는 사용자에게 그대로 노출된다):\n{reason}"),
    }
}

#[tokio::test]
#[ignore = "typescript-language-server 설치 필요"]
async fn references_finds_call_sites() {
    let root = repo_root();
    let rel = "src/lib/lsp.ts";
    let text = std::fs::read_to_string(root.join(rel)).expect("대상 파일 읽기");
    let (line, column) = locate(&text, "export function dedupeTargets", "dedupeTargets");

    let pool = LspPool::default();
    let targets = pool
        .goto(3, &root, rel, &text, line, column, GotoKind::References)
        .await
        .expect("사용처 조회 성공");

    println!("사용처 → {targets:?}");
    assert!(
        !targets.is_empty(),
        "resolveOutcome 등에서 쓰이므로 비어선 안 된다"
    );
    assert!(
        targets
            .iter()
            .all(|t| t.line != line || t.path.as_deref() != Some(rel)),
        "includeDeclaration=false 이므로 선언 자신은 빠져야 한다: {targets:?}",
    );
}

/// 대표 Gradle fixture를 읽기만 하며 jdtls의 import JVM 분리와 실제 탐색 결과를 확인한다.
/// 실행 래퍼가 `DOJANG_LSP_JAVA_ROOT`, `XDG_CACHE_HOME`, `GRADLE_USER_HOME`을 모두
/// disposable 경로로 공급한다. 원본 프로젝트를 직접 지정해 실행하지 않는다.
#[tokio::test]
#[ignore = "DOJANG_LSP_JAVA_ROOT·DOJANG_LSP_JAVA_PATH의 disposable Java fixture와 jdtls 필요"]
async fn java_navigation_definition_references_and_semantic_tokens() {
    let root =
        PathBuf::from(std::env::var("DOJANG_LSP_JAVA_ROOT").expect("DOJANG_LSP_JAVA_ROOT 필요"));
    let requested =
        std::env::var("DOJANG_LSP_JAVA_PATH").expect("DOJANG_LSP_JAVA_PATH 필요");
    let rel = if root.join(&requested).is_file() {
        requested
    } else {
        requested
            .strip_prefix("mirae/")
            .unwrap_or(&requested)
            .to_string()
    };
    let text = std::fs::read_to_string(root.join(&rel)).expect("FepClient fixture 읽기");
    let pool = LspPool::default();
    let result: Result<(), String> = async {
        let (type_line, type_column) = locate(&text, "private final FepApi fepApi;", "FepApi");
        let began = Instant::now();
        let type_targets = pool
            .goto(
                91,
                &root,
                &rel,
                &text,
                type_line,
                type_column,
                GotoKind::Definition,
            )
            .await?;
        println!(
            "FepApi definition {:?} in {:?}",
            began.elapsed(),
            type_targets
        );
        let fep_api = type_targets
            .iter()
            .find(|target| target.abs_path.ends_with("FepApi.java"))
            .ok_or_else(|| format!("FepApi.java 정의를 찾지 못했습니다: {type_targets:?}"))?;
        let fep_api_text = std::fs::read_to_string(&fep_api.abs_path)
            .map_err(|error| format!("FepApi 소스를 읽지 못했습니다: {error}"))?;
        let expected_type_line = locate(&fep_api_text, "interface FepApi", "FepApi").0;
        if fep_api.line != expected_type_line {
            return Err(format!(
                "FepApi 정의 줄이 다릅니다: 기대 {expected_type_line}, 실제 {}",
                fep_api.line
            ));
        }

        let (method_line, method_column) = locate(
            &text,
            "ClaimCountResponseDto response = fepApi.countClaims(",
            "countClaims",
        );
        let method_targets = pool
            .goto(
                91,
                &root,
                &rel,
                &text,
                method_line,
                method_column,
                GotoKind::Definition,
            )
            .await?;
        println!("countClaims definition {method_targets:?}");
        let count_claims = method_targets
            .iter()
            .find(|target| target.abs_path == fep_api.abs_path)
            .ok_or_else(|| format!("countClaims 정의를 찾지 못했습니다: {method_targets:?}"))?;
        let expected_method_line = locate(
            &fep_api_text,
            "ClaimCountResponseDto countClaims",
            "countClaims",
        )
        .0;
        if count_claims.line != expected_method_line {
            return Err(format!(
                "countClaims 정의 줄이 다릅니다: 기대 {expected_method_line}, 실제 {}",
                count_claims.line
            ));
        }

        let (refs_line, refs_column) = locate(&text, "public class FepClient", "FepClient");
        let began = Instant::now();
        let references = pool
            .goto(
                91,
                &root,
                &rel,
                &text,
                refs_line,
                refs_column,
                GotoKind::References,
            )
            .await?;
        println!("FepClient references {:?}: {references:?}", began.elapsed());
        if references.len() < 7 {
            return Err(format!(
                "FepClient 사용처가 7건보다 적습니다: {references:?}"
            ));
        }
        let unique = references
            .iter()
            .map(|target| format!("{}:{}:{}", target.abs_path, target.line, target.column))
            .collect::<HashSet<_>>();
        if unique.len() != references.len()
            || references.iter().any(|target| {
                target.path.as_deref() == Some(rel.as_str()) && target.line == refs_line
            })
        {
            return Err(format!(
                "사용처가 선언을 포함하거나 중복됩니다: {references:?}"
            ));
        }
        for expected in [
            "ClaimExternalService.java",
            "StatisticsService.java",
            "ContextTest.java",
        ] {
            if !references
                .iter()
                .any(|target| target.abs_path.ends_with(expected))
            {
                return Err(format!(
                    "{expected} 사용처를 찾지 못했습니다: {references:?}"
                ));
            }
        }

        let tokens = pool.semantic_tokens(91, &root, &rel, &text).await?;
        if tokens
            .as_ref()
            .map_or(true, |tokens| tokens.data.is_empty())
        {
            return Err("jdtls 의미 토큰이 비었습니다".into());
        }
        let status = pool.status_for(91, &root, &rel).await;
        if status.phase != "ready" {
            return Err(format!(
                "Java import 준비 상태가 ready가 아닙니다: {status:?}"
            ));
        }

        // 같은 scope를 재시작해도 disk cache를 다시 쓰며 탐색 결과를 보존해야 한다.
        pool.shutdown_task(91).await;
        let began = Instant::now();
        let warm_references = pool
            .goto(
                91,
                &root,
                &rel,
                &text,
                refs_line,
                refs_column,
                GotoKind::References,
            )
            .await?;
        println!(
            "FepClient warm references {:?}: {warm_references:?}",
            began.elapsed()
        );
        if warm_references.len() < 7 {
            return Err(format!(
                "warm restart 뒤 FepClient 사용처가 7건보다 적습니다: {warm_references:?}"
            ));
        }
        Ok(())
    }
    .await;
    pool.shutdown_all().await;
    result.expect("Java LSP 실제 탐색 검증 실패");
}

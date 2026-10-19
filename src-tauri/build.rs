fn main() {
    // `option_env!`는 컴파일 시점 값을 바이너리에 박는다. cargo는 이 매크로를 추적하지
    // 않으므로, 명시하지 않으면 **env를 바꿔도 재빌드가 일어나지 않아** 옛 client가
    // 그대로 남는다 (ADR 0147).
    println!("cargo:rerun-if-env-changed=PRAXIS_GMAIL_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=PRAXIS_GMAIL_CLIENT_SECRET");
    println!("cargo:rerun-if-env-changed=PRAXIS_ANTIGRAVITY_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=PRAXIS_ANTIGRAVITY_CLIENT_SECRET");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .plugin(
                "preview-bridge",
                tauri_build::InlinedPlugin::new().commands(&["submit_result"]),
            )
            .plugin(
                "preview-workbench",
                tauri_build::InlinedPlugin::new().commands(&["relay", "publish"]),
            ),
    )
    .expect("failed to generate preview bridge ACL");
}

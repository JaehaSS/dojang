use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn fixture() -> Prepared {
    let root = crate::testtmp::dir().join(format!(
        "pr-flow-{}",
        crate::preview_bridge::random_hex_id().unwrap()
    ));
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "Probe"]);
    git(&root, &["config", "user.email", "probe@example.invalid"]);
    std::fs::write(root.join("file"), "base").unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "base"],
    );
    let base_head = git(&root, &["rev-parse", "HEAD"]);
    git(&root, &["checkout", "-qb", "feature"]);
    std::fs::write(root.join("file"), "feature").unwrap();
    git(&root, &["add", "."]);
    git(
        &root,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "feature"],
    );
    let head = git(&root, &["rev-parse", "HEAD"]);
    let remote = root.join("local-remote.git");
    let output = Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(&remote)
        .output()
        .unwrap();
    assert!(output.status.success());
    git(
        &root,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&root, &["push", "-q", "origin", "feature"]);
    Prepared {
        request: DeliveryRequest {
            task_id: 7,
            worktree: root.clone(),
            branch: "feature".into(),
            base: "main".into(),
            verification_summary: "saved check".into(),
        },
        repository: "acme/widgets".into(),
        origin_url: remote.to_string_lossy().into_owned(),
        push_urls: vec![remote.to_string_lossy().into_owned()],
        canonical_worktree: root.canonicalize().unwrap(),
        head,
        base_head,
        title: "Frozen title".into(),
        body: "Frozen body\nsecond line\n".into(),
        template: None,
        expires_at: now() + 60,
    }
}
#[cfg(unix)]
fn gh_stub(value: &Prepared, existing: bool, fail: bool) -> PathBuf {
    let file = value.canonical_worktree.join("gh-stub.py");
    let metadata = serde_json::json!([{"number":7,"url":"https://github.com/acme/widgets/pull/7","state":"OPEN","headRefOid":value.head,"headRefName":"feature","baseRefName":"main","headRepository":{"nameWithOwner":"acme/widgets"}}]);
    let json_output = if existing {
        metadata.to_string()
    } else {
        "[]".into()
    };
    let script=format!("#!/usr/bin/env python3\nimport sys,pathlib,json\na=sys.argv[1:]\nr=pathlib.Path(__file__).parent\nif a[:2]==['pr','list']:\n print({json_output:?});sys.exit(0)\nassert a[:2] in [['pr','create'],['pr','edit']]\np=pathlib.Path(a[a.index('--body-file')+1])\n(r/'captured-body').write_bytes(p.read_bytes())\n(r/'captured-args').write_text(json.dumps(a))\n(r/'body-path').write_text(str(p))\nprint('https://github.com/acme/widgets/pull/7')\nsys.exit({})\n",if fail{1}else{0});
    std::fs::write(&file, script).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
    file
}
#[cfg(unix)]
#[test]
fn explicit_create_and_update_use_frozen_body_without_push() {
    for update in [false, true] {
        let value = fixture();
        let gh = gh_stub(&value, update, false);
        let result = create_or_update(&value, update, &gh).unwrap();
        assert_eq!(result.action, if update { "update" } else { "create" });
        assert_eq!(
            std::fs::read_to_string(value.canonical_worktree.join("captured-body")).unwrap(),
            value.body
        );
        let args: Vec<String> = serde_json::from_str(
            &std::fs::read_to_string(value.canonical_worktree.join("captured-args")).unwrap(),
        )
        .unwrap();
        assert_eq!(args[0], "pr");
        assert_eq!(args[1], if update { "edit" } else { "create" });
        assert!(args.windows(2).any(|v| v == ["--title", "Frozen title"]));
        assert!(!args.iter().any(|arg| arg == "--fill" || arg == "--dry-run"));
        let body_path =
            std::fs::read_to_string(value.canonical_worktree.join("body-path")).unwrap();
        assert!(!Path::new(&body_path).exists());
    }
}
#[cfg(unix)]
#[test]
fn remote_sha_mismatch_refuses_before_gh_mutation() {
    let mut value = fixture();
    let gh = gh_stub(&value, false, false);
    value.head = "a".repeat(40);
    assert!(create_or_update(&value, false, &gh)
        .unwrap_err()
        .contains("HEAD"));
    assert!(!value.canonical_worktree.join("captured-body").exists());
}
#[cfg(unix)]
#[test]
fn failed_gh_removes_body_and_body_is_owner_private() {
    let value = fixture();
    let gh = gh_stub(&value, false, true);
    assert!(create_or_update(&value, false, &gh).is_err());
    let path = std::fs::read_to_string(value.canonical_worktree.join("body-path")).unwrap();
    assert!(!Path::new(&path).exists());
    let body = BodyFile {
        path: write_body_file("private").unwrap(),
    };
    assert_eq!(
        std::fs::metadata(&body.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let at = body.path.clone();
    drop(body);
    assert!(!at.exists());
}
#[test]
fn captured_url_and_sha_ignore_later_origin_and_branch_changes() {
    let value = fixture();
    let alternate = value.canonical_worktree.join("other.git");
    let out = Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(&alternate)
        .output()
        .unwrap();
    assert!(out.status.success());
    git(
        &value.canonical_worktree,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            alternate.to_str().unwrap(),
        ],
    );
    std::fs::write(value.canonical_worktree.join("file"), "later").unwrap();
    git(&value.canonical_worktree, &["add", "."]);
    git(
        &value.canonical_worktree,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "later"],
    );
    push(&value).unwrap();
    let remote = git(
        &value.canonical_worktree,
        &["ls-remote", &value.push_urls[0], "refs/heads/feature"],
    );
    assert!(remote.starts_with(&value.head));
    assert!(git(
        &value.canonical_worktree,
        &[
            "ls-remote",
            alternate.to_str().unwrap(),
            "refs/heads/feature"
        ]
    )
    .is_empty());
}

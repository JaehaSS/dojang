#![cfg(target_os = "macos")]

use crate::knowledge::vault::selected_attachments::{
    cancel, consume_expected, has_explicit_snapshot, pending_policy, prepare, prepare_task_only,
    Selection,
};
use crate::knowledge::vault::{
    change_scope, create_text_source, migrate, register_project, register_vault, Scope,
    ScopeRequest, TextSourceDraft,
};

#[tokio::test]
async fn selected_snapshot_delivers_mixed_scopes_and_fails_closed() {
    let root = crate::testtmp::dir().join(format!("selected-attachments-{}", std::process::id()));
    let other_root =
        crate::testtmp::dir().join(format!("selected-attachments-other-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&other_root).unwrap();
    let pool = crate::knowledge::tests::test_pool().await;
    migrate(&pool).await.unwrap();
    let vault = register_vault(&pool, &root, 1).await.unwrap();
    let binding = register_project(&pool, &root, 1).await.unwrap();
    let other = register_project(&pool, &other_root, 1).await.unwrap();
    let root_text = root.to_string_lossy().into_owned();
    sqlx::query("INSERT INTO tasks (id, repo, branch, base, worktree_path, instruction, state, created_at, updated_at) VALUES (1, ?, 'main', 'base', ?, 'message', 'running', 1, 1)")
        .bind(&root_text)
        .bind(&root_text)
        .execute(&pool)
        .await
        .unwrap();
    let public = create_text_source(
        &pool,
        &TextSourceDraft {
            vault_id: vault.id.clone(),
            title: "project note".into(),
            body: "public selected excerpt".into(),
            scope: ScopeRequest {
                scope: Scope::Project {
                    key: binding.id.clone(),
                    binding_epoch: binding.epoch.clone(),
                },
            },
        },
        2,
    )
    .await
    .unwrap();
    let private = create_text_source(
        &pool,
        &TextSourceDraft {
            vault_id: vault.id.clone(),
            title: "private note".into(),
            body: "private selected excerpt".into(),
            scope: ScopeRequest {
                scope: Scope::PrivateData,
            },
        },
        3,
    )
    .await
    .unwrap();
    let selections = vec![
        Selection {
            revision_id: public.revision_id.clone(),
            expected_hash: public.sha256.clone(),
        },
        Selection {
            revision_id: private.revision_id.clone(),
            expected_hash: private.sha256.clone(),
        },
    ];

    let prepared = prepare(
        &pool,
        &binding,
        "message",
        "draft",
        &selections,
        "default",
        4,
    )
    .await
    .unwrap();
    assert_eq!(prepared.references.len(), 2);
    assert!(prepared
        .references
        .iter()
        .any(|item| item.scope == "project"));
    assert!(prepared
        .references
        .iter()
        .any(|item| item.scope == "private-data"));
    assert_eq!(
        pending_policy(&pool, &binding, "message", "draft")
            .await
            .unwrap()
            .unwrap()
            .policy
            .input_mode,
        "private_attachment"
    );
    assert!(pending_policy(&pool, &binding, "changed message", "draft")
        .await
        .is_err());
    assert!(
        prepare(&pool, &other, "message", "other", &selections, "default", 4)
            .await
            .is_err()
    );
    assert!(prepare(
        &pool,
        &binding,
        "message",
        "wrong-hash",
        &[Selection {
            revision_id: public.revision_id.clone(),
            expected_hash: "wrong".into(),
        }],
        "default",
        4,
    )
    .await
    .is_err());

    let delivered = consume_expected(&pool, &binding, "message", "draft", &prepared.preview.id, 1)
        .await
        .unwrap();
    assert_eq!(delivered.references.len(), 2);
    assert_eq!(
        delivered
            .references
            .iter()
            .find(|item| item.revision_id == private.revision_id)
            .unwrap()
            .snippet,
        "private selected excerpt"
    );
    assert!(
        consume_expected(&pool, &binding, "message", "draft", &prepared.preview.id, 2)
            .await
            .is_err()
    );
    cancel(&pool, &binding, "draft", "", 5).await.unwrap();
    assert!(!has_explicit_snapshot(&pool, "draft").await.unwrap());
    assert!(pending_policy(&pool, &binding, "message", "draft")
        .await
        .unwrap()
        .is_none());

    let stale_prepared = prepare(
        &pool,
        &binding,
        "message",
        "stale",
        &selections,
        "default",
        5,
    )
    .await
    .unwrap();
    change_scope(
        &pool,
        &private.revision_id,
        &ScopeRequest {
            scope: Scope::Common,
        },
        6,
    )
    .await
    .unwrap();
    assert!(consume_expected(
        &pool,
        &binding,
        "message",
        "stale",
        &stale_prepared.preview.id,
        1,
    )
    .await
    .is_err());

    prepare_task_only(&pool, &binding, "empty message", "task-only", 7)
        .await
        .unwrap();
    let task_only = crate::knowledge::vault::provenance::consume_draft_policy(
        &pool,
        &binding,
        "task-only",
        "empty message",
        1,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(task_only.input_mode, "task_only");
    assert!(task_only.sources.is_empty());
    assert!(!has_explicit_snapshot(&pool, "task-only").await.unwrap());
    cancel(&pool, &binding, "task-only", "", 8).await.unwrap();
    assert!(
        !crate::knowledge::vault::provenance::restrictive_draft_policy_seen(
            &pool,
            &binding,
            "task-only",
        )
        .await
        .unwrap()
    );

    let interleaved = prepare(
        &pool,
        &binding,
        "message",
        "interleaved",
        &selections,
        "default",
        9,
    )
    .await
    .unwrap();
    let accepted = pending_policy(&pool, &binding, "message", "interleaved")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(accepted.preview_id, interleaved.preview.id);
    cancel(&pool, &binding, "interleaved", "", 10)
        .await
        .unwrap();
    assert!(consume_expected(
        &pool,
        &binding,
        "message",
        "interleaved",
        &accepted.preview_id,
        1,
    )
    .await
    .is_err());

    prepare(
        &pool,
        &binding,
        "message",
        "cancel",
        &selections,
        "task_only",
        5,
    )
    .await
    .unwrap();
    cancel(&pool, &binding, "cancel", "message", 6)
        .await
        .unwrap();
    let pending_previews: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM vault_reference_previews WHERE client_ref = 'cancel' AND state = 'pending'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let pending_policies: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM vault_draft_policies WHERE client_ref = 'cancel' AND state = 'pending'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((pending_previews, pending_policies), (0, 1));
}

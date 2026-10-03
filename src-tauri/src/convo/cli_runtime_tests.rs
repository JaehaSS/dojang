use super::*;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "praxis-cli-test-{}",
            crate::convo::interaction::id().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn executable(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn fake_install(stage: &Path, version: &str) -> Result<(), String> {
    executable(
        &stage.join(ENTRY),
        &format!("#!/bin/sh\necho 'codex-cli {version}'\n"),
    );
    fs::write(stage.join("dependency"), "retained dependency").unwrap();
    Ok(())
}

#[test]
#[cfg(unix)]
fn global_update_and_removal_do_not_change_the_reopened_runtime() {
    let t = Temp::new();
    let global = t.0.join("global/codex");
    executable(
        &global,
        &format!("#!/bin/sh\necho 'codex-cli {DEFAULT_VERSION}'\n"),
    );
    let root = t.0.join("managed");
    let first = ensure_with(&root, DEFAULT_VERSION, |stage, version| {
        fake_install(stage, version)?;
        fs::copy(&global, stage.join(ENTRY)).map_err(err)?;
        Ok(())
    })
    .unwrap();
    executable(&global, "#!/bin/sh\necho 'codex-cli 9.0.0'\n");
    assert!(super::super::app_server::check_version(global.to_str().unwrap()).is_err());
    let reopened = ensure_with(&root, DEFAULT_VERSION, |_, _| panic!("must stay offline")).unwrap();
    assert_eq!(first, reopened);
    fs::remove_file(global).unwrap();
    assert_eq!(
        first,
        ensure_with(&root, DEFAULT_VERSION, |_, _| panic!("no global fallback")).unwrap()
    );
    super::super::app_server::check_version(&first).unwrap();
}

#[test]
#[cfg(unix)]
fn failed_or_wrong_version_install_is_not_published_and_retry_works() {
    let t = Temp::new();
    assert!(ensure_with(&t.0, DEFAULT_VERSION, |stage, _| {
        fs::write(stage.join("partial"), "partial").unwrap();
        Err("offline".into())
    })
    .unwrap_err()
    .contains("다시 시도"));
    assert!(!t.0.join(DEFAULT_VERSION).exists());
    assert!(ensure_with(&t.0, DEFAULT_VERSION, |stage, _| fake_install(
        stage, "9.0.0"
    ))
    .is_err());
    assert!(!t.0.join(DEFAULT_VERSION).exists());
    assert!(fs::read_dir(&t.0).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".install-")));
    ensure_with(&t.0, DEFAULT_VERSION, fake_install).unwrap();
}

#[test]
#[cfg(unix)]
fn approved_but_different_binary_version_cannot_be_published_under_another_pin() {
    let t = Temp::new();
    assert!(ensure_with(&t.0, DEFAULT_VERSION, |stage, _| fake_install(
        stage,
        LEGACY_VERSION
    ))
    .is_err());
    assert!(!t.0.join(DEFAULT_VERSION).exists());
    let old = ensure_with(&t.0, LEGACY_VERSION, fake_install).unwrap();
    let new = ensure_with(&t.0, DEFAULT_VERSION, fake_install).unwrap();
    assert_ne!(old, new);
    assert_eq!(
        old,
        ensure_with(&t.0, LEGACY_VERSION, |_, _| panic!(
            "legacy runtime replaced"
        ))
        .unwrap()
    );
}

#[test]
#[cfg(unix)]
fn dependency_damage_is_rejected_without_overwriting_the_runtime() {
    let t = Temp::new();
    ensure_with(&t.0, DEFAULT_VERSION, fake_install).unwrap();
    let dep = t.0.join(DEFAULT_VERSION).join("dependency");
    fs::write(&dep, "changed by another process").unwrap();
    let error = ensure_with(&t.0, DEFAULT_VERSION, |_, _| panic!("do not overwrite")).unwrap_err();
    assert!(error.contains("무결성"));
    assert_eq!(
        fs::read_to_string(dep).unwrap(),
        "changed by another process"
    );
}

#[test]
fn unapproved_version_never_reaches_filesystem_or_installer() {
    let t = Temp::new();
    let root = t.0.join("untouched");
    for version in ["9.0.0", "../../elsewhere", "", "0.154.0/other"] {
        assert!(ensure_with(&root, version, |_, _| panic!("unapproved")).is_err());
    }
    assert!(!root.exists());
}

#[test]
#[cfg(unix)]
fn external_links_are_rejected_but_internal_package_launcher_links_work() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let outside = t.0.join("outside");
    fs::write(&outside, "global dependency").unwrap();
    let root = t.0.join("managed");
    let error = ensure_with(&root, DEFAULT_VERSION, |stage, version| {
        fake_install(stage, version)?;
        symlink(&outside, stage.join("external")).unwrap();
        Ok(())
    })
    .unwrap_err();
    assert!(error.contains("외부"));
    assert!(!root.join(DEFAULT_VERSION).exists());
    let bin = ensure_with(&root, DEFAULT_VERSION, |stage, version| {
        fake_install(stage, version)?;
        fs::rename(stage.join(ENTRY), stage.join("launcher")).unwrap();
        symlink("../../launcher", stage.join(ENTRY)).unwrap();
        Ok(())
    })
    .unwrap();
    assert!(bin.ends_with("launcher"));
    assert_eq!(
        bin,
        ensure_with(&root, DEFAULT_VERSION, |_, _| panic!("cached")).unwrap()
    );
}

#[test]
#[cfg(unix)]
fn concurrent_preparations_publish_only_one_installation() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };
    let t = Temp::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let root = t.0.clone();
            let calls = calls.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                ensure_with(&root, DEFAULT_VERSION, |stage, version| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(50));
                    fake_install(stage, version)
                })
                .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results[0], results[1]);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
#[cfg(unix)]
fn version_probe_must_exit_successfully() {
    let t = Temp::new();
    assert!(ensure_with(&t.0, DEFAULT_VERSION, |stage, version| {
        executable(
            &stage.join(ENTRY),
            &format!("#!/bin/sh\necho 'codex-cli {version}'\nexit 1\n"),
        );
        Ok(())
    })
    .is_err());
    assert!(!t.0.join(DEFAULT_VERSION).exists());
}

#[test]
#[ignore = "Downloads the exact public npm package into a disposable directory; no model calls"]
fn real_package_install_and_offline_reopen() {
    let t = Temp::new();
    let bin = ensure(&t.0, DEFAULT_VERSION).unwrap();
    super::super::app_server::check_version(&bin).unwrap();
    let reopened = ensure_with(&t.0, DEFAULT_VERSION, |_, _| {
        panic!("must not download twice")
    })
    .unwrap();
    assert_eq!(bin, reopened);
}

#[test]
#[cfg(unix)]
fn installer_timeout_reaps_its_process_and_failure_returns_bounded_diagnostics() {
    let t = Temp::new();
    let pid_file = t.0.join("pid");
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "echo $$ > \"$PID_FILE\"; exec sleep 30"])
        .env("PID_FILE", &pid_file);
    assert!(
        run_install(command, &t.0.join("log"), Duration::from_millis(200))
            .unwrap_err()
            .contains("제한 시간")
    );
    let pid: u32 = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!crate::verify::process_group_alive(pid));
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "echo installation-failed >&2; exit 7"]);
    assert!(
        run_install(command, &t.0.join("log"), Duration::from_secs(2))
            .unwrap_err()
            .contains("installation-failed")
    );
}

#[test]
#[cfg(unix)]
fn question_and_resume_use_the_retained_package_after_global_update_and_db_restart() {
    use crate::convo::{
        app_server::{self, Context, Control},
        interaction as ledger, ConvoEvent, Vendor,
    };
    use std::sync::{atomic::Ordering, Arc};
    fn db<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }
    let t = Temp::new();
    let root = t.0.join("managed");
    let global = t.0.join("global-codex");
    executable(
        &global,
        include_str!("../../tests/fixtures/question_provider.py"),
    );
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(t.0.join("state.db"))
        .create_if_missing(true);
    for resume in [None, Some("test-thread")] {
        let p = db(sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone()))
        .unwrap();
        if resume.is_none() {
            db(sqlx::query("CREATE TABLE tasks(id INTEGER PRIMARY KEY,convo_session_id TEXT,pending_capsule TEXT);INSERT INTO tasks(id) VALUES(1)").execute(&p)).unwrap();
            db(ledger::migrate(&p)).unwrap();
            db(ledger::bind(&p, 1)).unwrap();
            // This fixture represents a pre-upgrade thread. Its recorded 0.154.0
            // contract must survive a newer default and global CLI replacement.
            db(
                sqlx::query("UPDATE convo_runtime_bindings SET cli_version=? WHERE task_id=1")
                    .bind(LEGACY_VERSION)
                    .execute(&p),
            )
            .unwrap();
        } else {
            db(ledger::migrate(&p)).unwrap();
        }
        let version = db(ledger::cli_version_of(&p, 1)).unwrap();
        assert_eq!(version, LEGACY_VERSION);
        let bin = ensure_with(&root, &version, |stage, _| {
            assert!(
                resume.is_none(),
                "resume must not access the global installation"
            );
            fs::create_dir_all(stage.join(ENTRY).parent().unwrap()).unwrap();
            fs::copy(&global, stage.join(ENTRY)).map_err(err)?;
            Ok(())
        })
        .unwrap();
        let ctx = Context {
            pool: p.clone(),
            task_id: 1,
            control: Arc::new(Control::new(
                db(ledger::begin(&p, 1, crate::now())).unwrap(),
                false,
            )),
            changed: Arc::new(|| {}),
        };
        let mut pid = 0;
        let mut answered = false;
        let result = app_server::run_selected(
            Some(&ctx),
            t.0.to_str().unwrap(),
            "normal",
            resume,
            5,
            Vendor::Codex,
            &bin,
            None,
            None,
            None,
            &[],
            None,
            None,
            |value| pid = value,
            |event| {
                if let ConvoEvent::Interaction { interaction_id } = event {
                    db(ledger::submit(
                        &p,
                        1,
                        &ctx.control.execution,
                        &interaction_id,
                        &ledger::id().unwrap(),
                        &[ledger::Answer {
                            question_id: "color".into(),
                            option_id: Some("blue".into()),
                            text: None,
                        }],
                        crate::now(),
                    ))
                    .unwrap();
                    answered = true;
                }
            },
        )
        .unwrap();
        assert_eq!(result.session_id, "test-thread");
        assert!(answered);
        assert!(!ctx.control.cleanup_failed.load(Ordering::SeqCst));
        assert!(pid > 0 && !crate::verify::process_group_alive(pid));
        drop(ctx);
        db(p.close());
        executable(&global, "#!/bin/sh\necho 'codex-cli 9.0.0'\nexit 1\n");
    }
}

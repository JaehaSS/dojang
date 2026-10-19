mod commands;
mod lsp;
mod open_path;
mod registry;
mod shell;
mod shell_commands;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use commands::*;
pub use lsp::*;
pub use open_path::*;
use registry::ProjectRegistry;
pub use registry::{ProjectEditorInfo, ProjectLaunch};
use shell::ProjectShell;
pub use shell_commands::*;

const SHARED_EDITOR_LABEL: &str = "editor";

/// The singleton editor receives its root from the main window. Keeping the
/// prepared roots separately from the active one lets the main window prepare
/// a task worktree before the editor has finished flushing its old buffers.
#[derive(Default)]
struct SharedProjectContext {
    authorized_roots: HashSet<PathBuf>,
    active_root: Option<PathBuf>,
}

#[derive(Default)]
pub struct ProjectEditorState {
    registry: Mutex<ProjectRegistry>,
    shared: Mutex<SharedProjectContext>,
    shared_binding: Mutex<()>,
    shells: Mutex<HashMap<String, Arc<Mutex<ProjectShell>>>>,
    write_guard: Mutex<()>,
    next_session: AtomicU64,
}

impl ProjectEditorState {
    fn root_for_window(&self, label: &str) -> Result<PathBuf, String> {
        let root = if label == SHARED_EDITOR_LABEL {
            self.shared
                .lock()
                .unwrap()
                .active_root
                .clone()
                .ok_or("공유 에디터에 활성 프로젝트가 없습니다")?
        } else {
            self.registry
                .lock()
                .unwrap()
                .root_for_label(label)
                .ok_or("등록되지 않은 프로젝트 창입니다")?
        };
        let current = root.canonicalize().map_err(|error| error.to_string())?;
        if current == root && current.is_dir() {
            Ok(root)
        } else {
            Err("프로젝트 루트가 변경되었거나 접근할 수 없습니다".into())
        }
    }

    fn root_for_request(
        &self,
        label: &str,
        expected_root: Option<&str>,
    ) -> Result<PathBuf, String> {
        let root = self.root_for_window(label)?;
        if let Some(expected_root) = expected_root {
            let expected_root = self.canonical_expected_root(label, expected_root)?;
            if expected_root != root {
                return Err("에디터의 프로젝트 루트가 변경되었습니다".into());
            }
        }
        Ok(root)
    }

    fn canonical_expected_root(&self, label: &str, root: &str) -> Result<PathBuf, String> {
        let canonical = registry::canonical_root(root)?;
        if label == SHARED_EDITOR_LABEL
            && registry::display_root(&canonical) != root.replace('\\', "/")
        {
            return Err("공유 에디터의 프로젝트 루트가 변경되었습니다".into());
        }
        Ok(canonical)
    }

    fn root_is_current(&self, label: &str, expected_root: &std::path::Path) -> bool {
        if label == SHARED_EDITOR_LABEL {
            self.shared.lock().unwrap().active_root.as_deref() == Some(expected_root)
        } else {
            self.registry
                .lock()
                .unwrap()
                .root_for_label(label)
                .as_deref()
                == Some(expected_root)
        }
    }

    fn prepare_shared_root(&self, root: PathBuf) {
        self.shared.lock().unwrap().authorized_roots.insert(root);
    }

    /// Returns whether the active root changed. A changed binding owns no
    /// previous shell, so a late shell open cannot survive a root switch.
    fn activate_shared_root(&self, root: Option<&str>) -> Result<bool, String> {
        let _binding = self.shared_binding.lock().unwrap();
        let next = match root {
            Some(root) => {
                let root = self.canonical_expected_root(SHARED_EDITOR_LABEL, root)?;
                let shared = self.shared.lock().unwrap();
                if !shared.authorized_roots.contains(&root) {
                    return Err("메인 창이 준비하지 않은 프로젝트 루트입니다".into());
                }
                Some(root)
            }
            None => None,
        };
        let mut shared = self.shared.lock().unwrap();
        let changed = shared.active_root != next;
        if changed {
            shared.active_root = next;
        }
        drop(shared);
        if changed {
            if let Some(shell) = self.shells.lock().unwrap().remove(SHARED_EDITOR_LABEL) {
                shell.lock().unwrap().terminate();
            }
        }
        Ok(changed)
    }

    fn clear_shared_context(&self) {
        let mut shared = self.shared.lock().unwrap();
        shared.active_root = None;
        shared.authorized_roots.clear();
    }

    fn next_session(&self) -> u64 {
        self.next_session.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// 창 label이 물고 있는 루트(표시 경로). 창이 닫혔음을 알릴 때 무엇이 닫혔는지 싣는다.
pub fn root_of_window(state: &ProjectEditorState, label: &str) -> Option<String> {
    let root = if label == SHARED_EDITOR_LABEL {
        state.shared.lock().unwrap().active_root.clone()?
    } else {
        state.registry.lock().unwrap().root_for_label(label)?
    };
    Some(registry::display_root(&root))
}

pub fn cleanup_window(state: &ProjectEditorState, label: &str) {
    let _binding = state.shared_binding.lock().unwrap();
    if label == SHARED_EDITOR_LABEL {
        state.clear_shared_context();
        if let Some(shell) = state.shells.lock().unwrap().remove(label) {
            shell.lock().unwrap().terminate();
        }
        return;
    }
    let opening = state.registry.lock().unwrap().remove_label(label);
    if opening.is_none() && !state.shells.lock().unwrap().contains_key(label) {
        return;
    }
    if let Some(opening) = opening {
        opening.complete(Err("프로젝트 창이 닫혔습니다".into()));
    }
    if let Some(shell) = state.shells.lock().unwrap().remove(label) {
        shell.lock().unwrap().terminate();
    }
}

pub fn cleanup_all(state: &ProjectEditorState) {
    // Closing main need not destroy other windows. Their roots stay registered
    // until their own Destroyed event, so editing and shell restart remain usable.
    let _binding = state.shared_binding.lock().unwrap();
    let shells: Vec<_> = state
        .shells
        .lock()
        .unwrap()
        .drain()
        .map(|(_, shell)| shell)
        .collect();
    for shell in shells {
        shell.lock().unwrap().terminate();
    }
}

fn insert_shell_if_registered(
    state: &ProjectEditorState,
    label: &str,
    root: &std::path::Path,
    shell: Arc<Mutex<ProjectShell>>,
) -> Option<u64> {
    let _binding = state.shared_binding.lock().unwrap();
    if !state.root_is_current(label, root) {
        return None;
    }
    let mut shells = state.shells.lock().unwrap();
    if let Some(current) = shells.get(label) {
        let current = current.lock().unwrap();
        if !current.exited() {
            return Some(current.session);
        }
    }
    let session = shell.lock().unwrap().session;
    shells.insert(label.to_string(), shell);
    Some(session)
}

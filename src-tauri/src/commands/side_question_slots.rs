use super::*;

/// Existing desktop tasks retain a slot until approval/deletion. An idle
/// conversation can lend that slot to its question; overlapping main/question
/// executions need two. Restored main turns may have no entry in `tasks`.
pub(super) fn occupied_slots(
    tasks: &HashMap<i64, ActiveTask>,
    main: &HashMap<i64, ActiveConvo>,
    questions: &HashSet<i64>,
) -> usize {
    tasks.len()
        + main.keys().filter(|id| !tasks.contains_key(id)).count()
        + questions
            .iter()
            .filter(|id| !tasks.contains_key(id) || main.contains_key(id))
            .count()
}

pub(super) struct QuestionSlot {
    active: Arc<Mutex<HashSet<i64>>>,
    task_id: i64,
}

impl Drop for QuestionSlot {
    fn drop(&mut self) {
        self.active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.task_id);
    }
}

// All admission paths take tasks -> main -> questions -> reserved. No lock is
// held across await, and neither cancellation map contains the other's key.
pub(super) fn reserve_question(state: &AppState, task_id: i64) -> Result<QuestionSlot, String> {
    refuse_while_updating(state)?;
    let tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner());
    let main = state.convo_active.lock().unwrap_or_else(|e| e.into_inner());
    let mut questions = state
        .side_question_active
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !questions.insert(task_id) {
        return Err("이미 답변 중인 질문이 있습니다".into());
    }
    let reserved = *state.reserved.lock().unwrap_or_else(|e| e.into_inner());
    let limit = state.max_concurrent.load(Ordering::Relaxed);
    if occupied_slots(&tasks, &main, &questions) + reserved > limit {
        questions.remove(&task_id);
        return Err(cap_reached_error(limit));
    }
    Ok(QuestionSlot {
        active: state.side_question_active.clone(),
        task_id,
    })
}

#[derive(Debug)]
enum MainSlotError {
    Busy(String),
    Capacity(usize),
}

fn reserve_main(
    state: &AppState,
    active: ActiveConvos,
    task_id: i64,
) -> Result<ConvoReservation, MainSlotError> {
    let tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner());
    let reservation = reserve_convo_switch(active.clone(), task_id).map_err(MainSlotError::Busy)?;
    let main = active.lock().unwrap_or_else(|e| e.into_inner());
    let questions = state
        .side_question_active
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let reserved = *state.reserved.lock().unwrap_or_else(|e| e.into_inner());
    let limit = state.max_concurrent.load(Ordering::Relaxed);
    let over_capacity =
        !questions.is_empty() && occupied_slots(&tasks, &main, &questions) + reserved > limit;
    drop(main);
    if over_capacity {
        // Drop releases only the main reservation we just acquired.
        return Err(MainSlotError::Capacity(limit));
    }
    // Keep the normal turn's existing handoff/drop ownership.
    Ok(reservation)
}

pub(super) async fn reserve_turn(
    state: &AppState,
    active: ActiveConvos,
    task_id: i64,
    initial: bool,
    uses_global_cli: bool,
) -> Result<ConvoReservation, String> {
    if !initial {
        if uses_global_cli {
            refuse_while_updating(state)?;
        }
        return reserve_main(state, active, task_id).map_err(|error| match error {
            MainSlotError::Busy(error) => error,
            MainSlotError::Capacity(limit) => cap_reached_error(limit),
        });
    }
    // InitialTask has no composer retry. Keep its main lane reserved while a
    // question returns borrowed capacity, so later sends cannot overtake it.
    let reservation = {
        let _tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner());
        reserve_convo_switch(active.clone(), task_id)?
    };
    loop {
        if uses_global_cli {
            refuse_while_updating(state)?;
        }
        let over_capacity = {
            let tasks = state.tasks.lock().unwrap_or_else(|e| e.into_inner());
            let main = active.lock().unwrap_or_else(|e| e.into_inner());
            let questions = state
                .side_question_active
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let reserved = *state.reserved.lock().unwrap_or_else(|e| e.into_inner());
            !questions.is_empty()
                && occupied_slots(&tasks, &main, &questions) + reserved
                    > state.max_concurrent.load(Ordering::Relaxed)
        };
        if !over_capacity {
            return Ok(reservation);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(limit: usize) -> AppState {
        let state = AppState::default();
        state.max_concurrent.store(limit, Ordering::Relaxed);
        state.tasks.lock().unwrap().insert(
            1,
            ActiveTask {
                worktree: Worktree {
                    repo: "/repo".into(),
                    path: "/worktree".into(),
                    branch: "question-test".into(),
                    base: "main".into(),
                    base_revision: None,
                },
                session: None,
                preview_mcp: None,
            },
        );
        state
    }

    #[test]
    fn main_and_question_overlap_in_either_order_and_release_independently() {
        for question_first in [false, true] {
            let state = state(2);
            let (main, question) = if question_first {
                let question = reserve_question(&state, 1).unwrap();
                (
                    reserve_main(&state, state.convo_active.clone(), 1).unwrap(),
                    question,
                )
            } else {
                let main = reserve_main(&state, state.convo_active.clone(), 1).unwrap();
                (main, reserve_question(&state, 1).unwrap())
            };
            assert!(reserve_question(&state, 1).is_err());
            assert!(reserve_question(&state, 2).is_err());
            assert!(reserve_main(&state, state.convo_active.clone(), 1).is_err());
            drop(question);
            assert!(state.convo_active.lock().unwrap().contains_key(&1));
            let question = reserve_question(&state, 1).unwrap();
            drop(main);
            assert!(state.side_question_active.lock().unwrap().contains(&1));
            drop(question);
            assert!(state.convo_active.lock().unwrap().is_empty());
            assert!(state.side_question_active.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn single_slot_can_be_lent_to_idle_question_but_never_runs_both() {
        let state = state(1);
        let main = reserve_main(&state, state.convo_active.clone(), 1).unwrap();
        assert!(reserve_question(&state, 1).is_err());
        drop(main);
        let question = reserve_question(&state, 1).unwrap();
        assert!(reserve_main(&state, state.convo_active.clone(), 1).is_err());
        assert!(state.convo_active.lock().unwrap().is_empty());
        drop(question);
        assert!(reserve_main(&state, state.convo_active.clone(), 1).is_ok());
    }

    #[test]
    fn pending_creation_restored_turns_and_runtime_limit_share_capacity() {
        let state = state(2);
        let main = reserve_main(&state, state.convo_active.clone(), 1).unwrap();
        let creation = reserve_slot(1, &state.reserved, 2).unwrap();
        assert!(reserve_question(&state, 1).is_err());
        drop(creation);
        let question = reserve_question(&state, 1).unwrap();
        let occupied = occupied_slots(
            &state.tasks.lock().unwrap(),
            &state.convo_active.lock().unwrap(),
            &state.side_question_active.lock().unwrap(),
        );
        assert!(reserve_slot(occupied, &state.reserved, 2).is_err());
        state.max_concurrent.store(1, Ordering::Relaxed);
        assert!(state.side_question_active.lock().unwrap().contains(&1));
        assert!(reserve_question(&state, 2).is_err());
        drop(question);
        drop(main);
        state.tasks.lock().unwrap().clear();
        let main = reserve_main(&state, state.convo_active.clone(), 99).unwrap();
        assert!(reserve_question(&state, 99).is_err());
        drop(main);
        let question = reserve_question(&state, 99).unwrap();
        assert!(active_work_count(&state) > 0);
        drop(question);
        assert_eq!(active_work_count(&state), 0);
        state.updating.store(true, Ordering::Relaxed);
        assert!(reserve_question(&state, 99).is_err());
    }

    #[tokio::test]
    async fn managed_turns_admit_during_global_update_but_global_turns_do_not() {
        let state = state(2);
        state.updating.store(true, Ordering::SeqCst);
        for initial in [true, false] {
            assert!(reserve_turn(&state, state.convo_active.clone(), 1, initial, true).await.is_err());
            let managed = reserve_turn(&state, state.convo_active.clone(), 1, initial, false).await.unwrap();
            assert!(state.convo_active.lock().unwrap().contains_key(&1));
            drop(managed);
            assert!(!state.convo_active.lock().unwrap().contains_key(&1));
        }
    }

    #[tokio::test]
    async fn initial_main_waits_for_a_lent_slot_without_losing_its_request() {
        let state = state(1);
        let question = reserve_question(&state, 1).unwrap();
        let start = reserve_turn(&state, state.convo_active.clone(), 1, true, true);
        tokio::pin!(start);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut start)
                .await
                .is_err()
        );
        assert!(state.convo_active.lock().unwrap().contains_key(&1));
        assert!(
            reserve_main(&state, state.convo_active.clone(), 1).is_err(),
            "later main sends cannot overtake the initial request"
        );
        drop(question);
        let main = tokio::time::timeout(std::time::Duration::from_secs(1), &mut start)
            .await
            .unwrap()
            .unwrap();
        assert!(state.convo_active.lock().unwrap().contains_key(&1));
        drop(main);
    }
}

use super::{DiffRange, FileDiff, ReviewSnapshot, Worktree};

impl Worktree {
    /// Reuse the exact displayed patches for hunk identities and annotation matching.
    pub fn review_snapshot(
        &self,
        files: &[FileDiff],
        range: DiffRange,
        patterns: &[String],
        stored: &[crate::annotations::ReviewAnnotation],
    ) -> anyhow::Result<ReviewSnapshot> {
        let mut hunks = files
            .iter()
            .flat_map(|file| crate::diffmodel::build_hunks(&file.patch, patterns))
            .collect::<Vec<_>>();
        if range == DiffRange::Session && !hunks.is_empty() {
            let pending = self.diff_detailed_range(DiffRange::Uncommitted)?;
            let pending = pending
                .iter()
                .flat_map(|file| crate::diffmodel::build_hunks(&file.patch, patterns))
                .collect::<Vec<_>>();
            for hunk in &mut hunks {
                hunk.committed = !pending
                    .iter()
                    .any(|other| crate::diffmodel::overlaps(hunk, other));
            }
        }
        let annotations = crate::annotations::rematch(stored, &hunks);
        Ok(ReviewSnapshot {
            hunks,
            annotations,
            warning: None,
        })
    }
}

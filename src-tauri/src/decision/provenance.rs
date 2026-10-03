use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Relation {
    Used,
    Generated,
    DerivedFrom,
    ApprovedBy,
    Supersedes,
    BlockedBy,
}

impl Relation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Used => "used",
            Self::Generated => "generated",
            Self::DerivedFrom => "derived_from",
            Self::ApprovedBy => "approved_by",
            Self::Supersedes => "supersedes",
            Self::BlockedBy => "blocked_by",
        }
    }
}

/// 쓰는 쪽의 종류만 둔다. 1.0 이전 기록의 `task_start_receipt`·`memory_version`·`evidence_check`
/// 링크는 스키마 CHECK가 여전히 허용하고, 읽는 쪽(`why_trace`)은 문자열 그대로 순회한다.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArtifactKind {
    Task,
    InstructionDigest,
    VerificationRun,
    GitCommit,
    Actor,
}

impl ArtifactKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::InstructionDigest => "instruction_digest",
            Self::VerificationRun => "verification_run",
            Self::GitCommit => "git_commit",
            Self::Actor => "actor",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactLink {
    pub relation: Relation,
    pub kind: ArtifactKind,
    pub artifact_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalProvenance {
    pub task_id: i64,
    pub instruction_digest: String,
    pub verification_run: Option<String>,
    pub commit_sha: String,
}

impl ApprovalProvenance {
    pub fn decision_key_hash(&self) -> anyhow::Result<String> {
        validate_hex("instruction digest", &self.instruction_digest, 64, 64)?;
        super::approval_journal::validate_commit_sha(&self.commit_sha)?;
        Ok(digest(&format!(
            "task_approval:{}:{}",
            self.task_id, self.commit_sha
        )))
    }

    pub fn links(&self) -> anyhow::Result<Vec<ArtifactLink>> {
        super::provenance_links::links(self)
    }
}

pub fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn verification_ref(task_id: i64, created_at: i64) -> String {
    digest(&format!("task_evidence:{task_id}:{created_at}"))
}

pub(super) fn validate_hex(label: &str, value: &str, min: usize, max: usize) -> anyhow::Result<()> {
    if !(min..=max).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        anyhow::bail!("{label} is not a bounded hexadecimal identity");
    }
    Ok(())
}

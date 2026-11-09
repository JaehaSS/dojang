//! GitHub issue discovery and task linkage through the `gh` CLI.

use serde::{Deserialize, Serialize};

mod remote;
pub mod pr_status;
pub mod pr_delivery;
mod store;

pub use remote::{
    delete_issue, is_github_remote, list_issues, parse_owner_repo, pr_url_for_branch,
    remote_owner_repo, resolve_repos, view_issue,
};
pub use store::{get_issue_ref, get_pr_ref, migrate, set_issue_ref, set_pr_ref};

#[cfg(test)]
use remote::{
    delete_issue_with_bin, list_issues_with_bin, looks_unauthenticated, parse_issue_detail,
    parse_issue_list, view_issue_with_bin,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhError {
    GhUnavailable,
    CommandFailed(String),
}

impl std::fmt::Display for GhError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GhUnavailable => write!(formatter, "gh CLI를 사용할 수 없습니다"),
            Self::CommandFailed(message) => write!(formatter, "{message}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhLabel {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhIssue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub labels: Vec<GhLabel>,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

/// 이슈를 볼 수 있는 레포 하나 — 로컬 경로와 그 경로가 가리키는 `owner/repo`.
/// 홈의 레포 전환 버튼이 "어느 레포인지"를 경로가 아니라 이 이름으로 보여준다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhRepo {
    pub path: String,
    pub owner_repo: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhIssueDetail {
    pub title: String,
    #[serde(default)]
    pub body: String,
}

pub fn build_instruction(owner_repo: &str, number: u64, detail: &GhIssueDetail) -> String {
    let title = detail.title.trim();
    let body = detail.body.trim();
    if body.is_empty() {
        return format!("{title}\n\n(GitHub 이슈 {owner_repo}#{number})");
    }
    format!("{title}\n\n{body}\n\n(GitHub 이슈 {owner_repo}#{number})")
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod pr_status_tests;

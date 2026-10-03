use super::provenance::{validate_hex, ApprovalProvenance, ArtifactKind, ArtifactLink, Relation};

pub(super) fn links(provenance: &ApprovalProvenance) -> anyhow::Result<Vec<ArtifactLink>> {
    provenance.decision_key_hash()?;
    let mut links = fixed_links(provenance);
    append_verification(&mut links, provenance.verification_run.as_deref())?;
    sort_and_deduplicate(&mut links);
    Ok(links)
}

fn fixed_links(provenance: &ApprovalProvenance) -> Vec<ArtifactLink> {
    vec![
        link(
            Relation::ApprovedBy,
            ArtifactKind::Actor,
            "local-human".into(),
        ),
        link(
            Relation::DerivedFrom,
            ArtifactKind::Task,
            provenance.task_id.to_string(),
        ),
        link(
            Relation::Used,
            ArtifactKind::InstructionDigest,
            provenance.instruction_digest.clone(),
        ),
        link(
            Relation::Generated,
            ArtifactKind::GitCommit,
            provenance.commit_sha.clone(),
        ),
    ]
}

fn append_verification(
    links: &mut Vec<ArtifactLink>,
    reference: Option<&str>,
) -> anyhow::Result<()> {
    let Some(reference) = reference else {
        return Ok(());
    };
    validate_hex("verification run", reference, 64, 64)?;
    links.push(link(
        Relation::Used,
        ArtifactKind::VerificationRun,
        reference.into(),
    ));
    Ok(())
}

fn sort_and_deduplicate(links: &mut Vec<ArtifactLink>) {
    links.sort_by(|left, right| {
        (
            left.kind.as_str(),
            &left.artifact_ref,
            left.relation.as_str(),
        )
            .cmp(&(
                right.kind.as_str(),
                &right.artifact_ref,
                right.relation.as_str(),
            ))
    });
    links.dedup();
}

fn link(relation: Relation, kind: ArtifactKind, artifact_ref: String) -> ArtifactLink {
    ArtifactLink {
        relation,
        kind,
        artifact_ref,
    }
}

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

use chrono::{Duration, Utc};
use mcp_context_pack::{
    adapters::{code_excerpt_fs::CodeExcerptFsAdapter, storage_json::JsonStorageAdapter},
    app::{
        input_usecases::{InputUseCases, TouchTtlMode, UpsertRefRequest},
        output_usecases::{OutputProfile, OutputReadRequest, OutputUseCases},
        ports::FreshnessState,
    },
    domain::errors::DomainError,
    domain::models::Pack,
    domain::types::{PackId, PackName, Status},
};

fn build_services(
    storage_dir: PathBuf,
    source_root: PathBuf,
) -> (Arc<InputUseCases>, Arc<OutputUseCases>) {
    let storage = Arc::new(JsonStorageAdapter::new(storage_dir));
    let excerpts = Arc::new(CodeExcerptFsAdapter::new(source_root.clone()).unwrap());
    let input_uc = Arc::new(InputUseCases::new(storage.clone(), excerpts.clone()));
    let output_uc = Arc::new(OutputUseCases::new(storage, excerpts));
    (input_uc, output_uc)
}

fn oversized_pack_for_limit(id: PackId, max_bytes: usize) -> Pack {
    let pack_id = id.clone();
    let mut pack = Pack::new(
        id,
        Some(PackName::new(&format!("oversized-{pack_id}")).unwrap()),
    );
    pack.brief = Some("x".repeat(max_bytes + 1));
    pack
}

fn make_named_pack_with(
    name: &str,
    status: Status,
    updated_at: chrono::DateTime<Utc>,
    revision: u64,
) -> Pack {
    let mut pack = Pack::new(PackId::new(), Some(PackName::new(name).unwrap()));
    pack.status = status;
    pack.updated_at = updated_at;
    pack.created_at = updated_at - Duration::minutes(1);
    pack.revision = revision.max(1);
    pack.expires_at = updated_at + Duration::hours(24);
    pack
}

fn write_pack_file(storage_dir: &std::path::Path, pack: &Pack) {
    std::fs::create_dir_all(storage_dir).unwrap();
    let payload = serde_json::to_string(pack).unwrap();
    std::fs::write(
        storage_dir.join(format!("{}.json", pack.id.as_str())),
        payload,
    )
    .unwrap();
}

#[tokio::test]
async fn test_full_pack_lifecycle() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");

    // Create a dummy source file with known content
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(
        source_root.join("sample.rs"),
        "line1\nline2\nline3\nline4\nline5\n",
    )
    .unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());

    // Create pack
    let pack = input_uc
        .create_with_tags_ttl(
            Some("my-pack".into()),
            Some("My Test Pack".into()),
            Some("a brief".into()),
            None,
            30,
        )
        .await
        .unwrap();
    let pack_id = pack.id.as_str().to_string();
    let mut revision = pack.revision;

    // Add required scope section
    let pack = input_uc
        .upsert_section_checked(
            &pack_id,
            "scope",
            "Scope".into(),
            Some("This pack covers auth flow".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    // Add required findings section
    let pack = input_uc
        .upsert_section_checked(
            &pack_id,
            "findings",
            "Findings".into(),
            Some("Critical findings".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    // Add finding ref
    let pack = input_uc
        .upsert_ref_checked(
            &pack_id,
            UpsertRefRequest {
                section_key: "findings".into(),
                ref_key: "ref-one".into(),
                path: "src/sample.rs".into(),
                line_start: 2,
                line_end: 3,
                title: Some("My Ref".into()),
                why: Some("important context".into()),
                group: None,
            },
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    // Add required QA section with verdict field
    let pack = input_uc
        .upsert_section_checked(
            &pack_id,
            "qa",
            "QA".into(),
            Some("verdict: pass".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    // Finalize
    input_uc
        .set_status_checked(&pack_id, Status::Finalized, revision)
        .await
        .unwrap();

    // Render output with reviewer profile to validate full evidence excerpts
    let rendered = output_uc
        .get_rendered_with_request(
            &pack_id,
            OutputReadRequest {
                profile: Some(OutputProfile::Reviewer),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert!(rendered.contains("[LEGEND]"), "must have LEGEND section");
    assert!(rendered.contains("[CONTENT]"), "must have CONTENT section");
    assert!(rendered.contains("# Context pack: My Test Pack"));
    assert!(rendered.contains("## Findings"));
    assert!(rendered.contains("line2"), "line2 must be in excerpt");
    assert!(rendered.contains("line3"), "line3 must be in excerpt");
    assert!(!rendered.contains("line1"), "line1 must NOT be in excerpt");
    assert!(!rendered.contains("line4"), "line4 must NOT be in excerpt");
}

#[tokio::test]
async fn test_duplicate_name_is_rejected() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    input_uc
        .create_with_tags_ttl(Some("unique-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let res = input_uc
        .create_with_tags_ttl(Some("unique-pack".into()), None, None, None, 30)
        .await;
    assert!(res.is_err(), "duplicate name must fail");
}

#[tokio::test]
async fn test_finalize_empty_pack_rejected() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("emp-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let res = input_uc
        .set_status_checked(pack.id.as_str(), Status::Finalized, pack.revision)
        .await;
    assert!(
        matches!(
            res,
            Err(DomainError::FinalizeValidation {
                missing_sections,
                missing_fields,
                ..
            }) if missing_sections == vec![
                "scope".to_string(),
                "findings".to_string(),
                "qa".to_string()
            ] && missing_fields.is_empty()
        ),
        "cannot finalize empty pack: required sections must be reported"
    );
}

#[tokio::test]
async fn test_finalize_pack_with_only_empty_section_rejected() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("emp-sec-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(&id, "sec-one", "S".into(), None, None, pack.revision)
        .await
        .unwrap();
    let res = input_uc
        .set_status_checked(&id, Status::Finalized, pack.revision)
        .await;
    assert!(
        matches!(
            res,
            Err(DomainError::FinalizeValidation {
                missing_sections,
                ..
            }) if missing_sections == vec![
                "scope".to_string(),
                "findings".to_string(),
                "qa".to_string()
            ]
        ),
        "cannot finalize pack with unrelated sections only"
    );
}

#[tokio::test]
async fn test_finalize_missing_qa_or_verdict_is_actionable() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("sample.rs"), "line1\n").unwrap();

    let (input_uc, _) = build_services(storage_dir, tmp.path().to_path_buf());
    let pack = input_uc
        .create_with_tags_ttl(Some("qa-check-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let mut revision = pack.revision;

    let pack = input_uc
        .upsert_section_checked(
            &id,
            "scope",
            "Scope".into(),
            Some("scope coverage".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    let pack = input_uc
        .upsert_section_checked(
            &id,
            "findings",
            "Findings".into(),
            Some("finding summary".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "findings".into(),
                ref_key: "finding-ref".into(),
                path: "src/sample.rs".into(),
                line_start: 1,
                line_end: 1,
                title: None,
                why: None,
                group: None,
            },
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    let missing_qa = input_uc
        .set_status_checked(&id, Status::Finalized, revision)
        .await;
    assert!(
        matches!(
            missing_qa,
            Err(DomainError::FinalizeValidation {
                missing_sections,
                missing_fields,
                ..
            }) if missing_sections == vec!["qa".to_string()] && missing_fields.is_empty()
        ),
        "missing qa section must be reported explicitly"
    );

    let pack = input_uc
        .upsert_section_checked(
            &id,
            "qa",
            "QA".into(),
            Some("checks run".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    let missing_verdict = input_uc
        .set_status_checked(&id, Status::Finalized, revision)
        .await;
    assert!(
        matches!(
            missing_verdict,
            Err(DomainError::FinalizeValidation {
                missing_sections,
                missing_fields,
                ..
            }) if missing_sections.is_empty()
                && missing_fields == vec!["qa.verdict".to_string()]
        ),
        "qa.verdict field must be enforced at finalize"
    );
}

#[tokio::test]
async fn test_draft_workflow_remains_flexible_before_finalize() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("draft.rs"), "fn probe() {}\n").unwrap();

    let (input_uc, _) = build_services(storage_dir, tmp.path().to_path_buf());
    let pack = input_uc
        .create_with_tags_ttl(Some("draft-flex-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();

    let pack = input_uc
        .upsert_section_checked(
            &id,
            "notes",
            "Exploration notes".into(),
            Some("still drafting".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "notes".into(),
                ref_key: "probe-ref".into(),
                path: "src/draft.rs".into(),
                line_start: 1,
                line_end: 1,
                title: None,
                why: None,
                group: None,
            },
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .set_meta_checked(
            &id,
            Some("Draft in progress".into()),
            Some("No finalize sections yet".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();

    assert_eq!(pack.status, Status::Draft);
    assert!(
        pack.sections
            .iter()
            .any(|section| section.key.as_str() == "notes"),
        "draft mutations should remain allowed without finalize checklist sections"
    );
}

#[tokio::test]
async fn test_set_meta_empty_payload_rejected() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());
    let pack = input_uc
        .create_with_tags_ttl(Some("meta-empty".into()), None, None, None, 30)
        .await
        .unwrap();
    let res = input_uc
        .set_meta_checked(pack.id.as_str(), None, None, None, pack.revision)
        .await;
    assert!(res.is_err(), "set_meta must require at least one field");
}

#[tokio::test]
async fn test_delete_section() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("sect-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(&id, "sec-one", "S".into(), None, None, pack.revision)
        .await
        .unwrap();
    let pack = input_uc
        .delete_section_checked(&id, "sec-one", pack.revision)
        .await
        .unwrap();
    assert!(pack.sections.is_empty(), "section must be deleted");
}

#[tokio::test]
async fn test_list_with_query() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    input_uc
        .create_with_tags_ttl(
            Some("alpha-pack".into()),
            Some("Alpha Title".into()),
            None,
            None,
            30,
        )
        .await
        .unwrap();
    input_uc
        .create_with_tags_ttl(
            Some("beta-pack".into()),
            Some("Beta Title".into()),
            None,
            None,
            30,
        )
        .await
        .unwrap();

    let results = input_uc
        .list(None, Some("Alpha".into()), None, None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1, "only one pack should match 'Alpha'");
    assert_eq!(results[0].name.as_ref().unwrap().as_str(), "alpha-pack");
}

#[tokio::test]
async fn test_list_pagination() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    for i in 1..=4 {
        input_uc
            .create_with_tags_ttl(Some(format!("pack-{i:02}")), None, None, None, 30)
            .await
            .unwrap();
    }

    let all = input_uc.list(None, None, None, None).await.unwrap();
    assert_eq!(all.len(), 4);

    let page = input_uc.list(None, None, Some(2), Some(1)).await.unwrap();
    assert_eq!(page.len(), 2, "limit=2 offset=1 should give 2 items");
}

#[tokio::test]
async fn test_name_resolution_is_deterministic_and_prefers_finalized() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("ctx.rs"), "fn stable() {}\n").unwrap();

    let (input_uc, output_uc) = build_services(storage_dir.clone(), tmp.path().to_path_buf());
    let now = Utc::now();

    let selected = make_named_pack_with("det-name", Status::Finalized, now, 8);
    let older_finalized = make_named_pack_with(
        "det-name",
        Status::Finalized,
        now - Duration::minutes(10),
        10,
    );
    let newer_draft =
        make_named_pack_with("det-name", Status::Draft, now + Duration::minutes(1), 99);

    write_pack_file(&storage_dir, &selected);
    write_pack_file(&storage_dir, &older_finalized);
    write_pack_file(&storage_dir, &newer_draft);

    let first = input_uc.get("det-name").await.unwrap();
    let second = input_uc.get("det-name").await.unwrap();
    assert_eq!(
        first.id, selected.id,
        "lookup must prefer latest finalized candidate"
    );
    assert_eq!(
        second.id, selected.id,
        "same input must resolve to the same pack under unchanged state"
    );

    let rendered = output_uc.get_rendered("det-name", None).await.unwrap();
    assert_eq!(
        legend_value(&rendered, "id").as_deref(),
        Some(selected.id.as_str()),
        "output rendering by name should resolve to deterministic winner"
    );
}

#[tokio::test]
async fn test_name_resolution_ambiguity_exposes_candidate_ids() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();

    let (input_uc, output_uc) = build_services(storage_dir.clone(), tmp.path().to_path_buf());
    let shared_time = Utc::now();
    let candidate_a = make_named_pack_with("ambig-name", Status::Finalized, shared_time, 11);
    let candidate_b = make_named_pack_with("ambig-name", Status::Finalized, shared_time, 11);

    write_pack_file(&storage_dir, &candidate_a);
    write_pack_file(&storage_dir, &candidate_b);

    let input_err = input_uc.get("ambig-name").await.unwrap_err();
    match input_err {
        DomainError::Ambiguous { mut candidates, .. } => {
            candidates.sort();
            let mut expected = vec![
                candidate_a.id.as_str().to_string(),
                candidate_b.id.as_str().to_string(),
            ];
            expected.sort();
            assert_eq!(candidates, expected);
        }
        other => panic!("expected DomainError::Ambiguous, got: {:?}", other),
    }

    let output_err = output_uc
        .get_rendered("ambig-name", None)
        .await
        .unwrap_err();
    assert!(
        matches!(output_err, DomainError::Ambiguous { .. }),
        "output lookup must fail closed on ambiguity"
    );
}

#[tokio::test]
async fn test_revision_increments() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("rev-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    assert_eq!(pack.revision, 1);

    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(&id, "sec-one", "S".into(), None, None, pack.revision)
        .await
        .unwrap();
    assert_eq!(
        pack.revision, 2,
        "revision must increment on upsert_section"
    );

    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "sec-one".into(),
                ref_key: "ref-one".into(),
                path: "Cargo.toml".into(),
                line_start: 1,
                line_end: 2,
                title: None,
                why: None,
                group: None,
            },
            pack.revision,
        )
        .await
        .unwrap();
    assert_eq!(pack.revision, 3, "revision must increment on upsert_ref");
}

#[tokio::test]
async fn test_json_round_trip() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(
            Some("json-pack".into()),
            Some("Json Pack".into()),
            Some("brief text".into()),
            None,
            30,
        )
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "sec-one",
            "S".into(),
            Some("desc".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    input_uc
        .set_meta_checked(
            &id,
            None,
            None,
            Some(vec!["tag1".into(), "tag2".into()]),
            pack.revision,
        )
        .await
        .unwrap();

    // Re-fetch from disk
    let loaded = input_uc.get(&id).await.unwrap();
    assert_eq!(loaded.title.as_deref(), Some("Json Pack"));
    assert_eq!(loaded.tags, vec!["tag1", "tag2"]);
    assert_eq!(loaded.sections.len(), 1);
    assert_eq!(loaded.sections[0].description.as_deref(), Some("desc"));
}

#[tokio::test]
async fn test_create_applies_tags() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(
            Some("tagged-pack".into()),
            Some("Tagged Pack".into()),
            Some("brief".into()),
            Some(vec!["mcp".into(), "qa".into()]),
            30,
        )
        .await
        .unwrap();

    assert_eq!(pack.tags, vec!["mcp", "qa"]);
}

#[tokio::test]
async fn test_stale_ref_in_output() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    // File with only 2 lines
    std::fs::write(source_root.join("short.rs"), "line1\nline2\n").unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("stale-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "scope",
            "Scope".into(),
            Some("scope context".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "findings",
            "Findings".into(),
            Some("finding summary".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    // Ref points to lines 10-20 (past end of 2-line file)
    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "findings".into(),
                ref_key: "ref-one".into(),
                path: "src/short.rs".into(),
                line_start: 10,
                line_end: 20,
                title: None,
                why: None,
                group: None,
            },
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "qa",
            "QA".into(),
            Some("verdict: fail".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    let finalize = input_uc
        .set_status_checked(&id, Status::Finalized, pack.revision)
        .await;
    assert!(
        matches!(
            finalize,
            Err(DomainError::FinalizeValidation { invalid_refs, .. })
            if !invalid_refs.is_empty()
                && invalid_refs[0].section_key == "findings"
                && invalid_refs[0].ref_key == "ref-one"
        ),
        "finalize must fail-closed with actionable invalid_refs details"
    );

    let rendered = output_uc.get_rendered(&id, None).await.unwrap();
    assert!(
        rendered.contains("ref-one"),
        "draft output should still include ref metadata"
    );
}

#[tokio::test]
async fn test_checked_mutation_rejects_stale_revision() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("conflict-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let revision = pack.revision;

    let updated = input_uc
        .upsert_section_checked(&id, "sec-one", "S".into(), None, None, revision)
        .await
        .unwrap();
    assert_eq!(updated.revision, revision + 1);

    let stale = input_uc
        .set_meta_checked(&id, Some("X".into()), None, None, revision)
        .await;
    match stale {
        Err(DomainError::RevisionConflictDetailed {
            expected_revision,
            current_revision,
            last_updated_at,
            changed_section_keys,
            guidance,
        }) => {
            assert_eq!(expected_revision, revision);
            assert_eq!(current_revision, revision + 1);
            assert!(
                !last_updated_at.is_empty(),
                "last_updated_at must be included"
            );
            assert!(
                changed_section_keys.iter().any(|key| key == "sec-one"),
                "changed_section_keys should include changed section"
            );
            assert!(
                guidance.contains("re-read latest pack") && guidance.contains("expected_revision"),
                "guidance must explain safe retry workflow"
            );
        }
        other => panic!("expected RevisionConflictDetailed, got: {other:?}"),
    }

    let after_conflict = input_uc.get(&id).await.unwrap();
    assert_eq!(
        after_conflict.title, None,
        "conflict must remain fail-closed without partial overwrite"
    );

    let retried = input_uc
        .set_meta_checked(
            &id,
            Some("X".into()),
            Some("retry after reread".into()),
            None,
            after_conflict.revision,
        )
        .await
        .expect("retry after reread should succeed");
    assert_eq!(retried.title.as_deref(), Some("X"));
}

#[tokio::test]
async fn test_revision_conflict_changed_section_keys_are_bounded() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("conflict-bounded-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let stale_revision = pack.revision;
    let mut current_revision = pack.revision;

    for i in 0..20 {
        let key = format!("sec-{i:02}");
        let updated = input_uc
            .upsert_section_checked(
                &id,
                &key,
                format!("Section {i:02}"),
                Some("agent A update".into()),
                None,
                current_revision,
            )
            .await
            .unwrap();
        current_revision = updated.revision;
    }

    let stale = input_uc
        .set_meta_checked(&id, Some("stale".into()), None, None, stale_revision)
        .await;
    match stale {
        Err(DomainError::RevisionConflictDetailed {
            expected_revision,
            current_revision: conflict_current,
            changed_section_keys,
            ..
        }) => {
            assert_eq!(expected_revision, stale_revision);
            assert_eq!(conflict_current, current_revision);
            assert!(
                changed_section_keys.len() <= 12,
                "changed_section_keys must be bounded"
            );
            assert!(
                changed_section_keys.first().is_some_and(|k| k == "sec-00"),
                "deterministic ordering should keep sorted section keys"
            );
        }
        other => panic!("expected bounded RevisionConflictDetailed, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_touch_ttl_updates_revision_and_legend() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");

    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("main.rs"), "fn main() {}\n").unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("ttl-pack".into()), None, None, None, 5)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let revision = pack.revision;

    let touched = input_uc
        .touch_ttl_checked(&id, revision, TouchTtlMode::ExtendMinutes(30))
        .await
        .unwrap();
    assert_eq!(touched.revision, revision + 1);

    let rendered = output_uc.get_rendered(&id, None).await.unwrap();
    assert!(
        rendered.contains("ttl_remaining"),
        "legend must include ttl"
    );
    assert!(
        rendered.contains("expires_at"),
        "legend must include expiry"
    );
}

#[tokio::test]
async fn test_upsert_section_update_preserves_order_without_explicit_order() {
    let tmp = tempdir().unwrap();
    let (input_uc, _) = build_services(tmp.path().join("packs"), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("order-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let mut revision = pack.revision;

    for key in ["aa", "bb", "cc"] {
        let updated = input_uc
            .upsert_section_checked(&id, key, key.to_uppercase(), None, None, revision)
            .await
            .unwrap();
        revision = updated.revision;
    }

    let updated = input_uc
        .upsert_section_checked(&id, "aa", "AA+".into(), None, None, revision)
        .await
        .unwrap();
    let keys: Vec<&str> = updated.sections.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(keys, vec!["aa", "bb", "cc"]);
}

#[tokio::test]
async fn test_finalize_rejects_ref_when_line_end_exceeds_file_length() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("short.rs"), "line1\nline2\n").unwrap();

    let (input_uc, _) = build_services(storage_dir, tmp.path().to_path_buf());
    let pack = input_uc
        .create_with_tags_ttl(Some("end-stale".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "scope",
            "Scope".into(),
            Some("scope coverage".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "findings",
            "Findings".into(),
            Some("finding summary".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "findings".into(),
                ref_key: "ref-one".into(),
                path: "src/short.rs".into(),
                line_start: 1,
                line_end: 99,
                title: None,
                why: None,
                group: None,
            },
            pack.revision,
        )
        .await
        .unwrap();
    let pack = input_uc
        .upsert_section_checked(
            &id,
            "qa",
            "QA".into(),
            Some("verdict: fail".into()),
            None,
            pack.revision,
        )
        .await
        .unwrap();

    let finalize = input_uc
        .set_status_checked(&id, Status::Finalized, pack.revision)
        .await;
    assert!(
        matches!(
            finalize,
            Err(DomainError::FinalizeValidation { invalid_refs, .. })
            if invalid_refs.iter().any(|issue| issue.ref_key == "ref-one")
        ),
        "finalize must fail with ref-level details when line_end is stale"
    );
}

#[tokio::test]
async fn test_malformed_pack_file_is_recovered_by_list() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let malformed_path = storage_dir.join("pk_aaaaaaaa.json");
    std::fs::write(&malformed_path, "not-json").unwrap();

    let (input_uc, _) = build_services(storage_dir, tmp.path().to_path_buf());
    let listed = input_uc.list(None, None, None, None).await;
    assert!(
        matches!(listed, Ok(items) if items.is_empty()),
        "expected malformed pack to be skipped"
    );
    assert!(
        !malformed_path.exists(),
        "malformed pack should be removed during lookup/list recovery"
    );
}

#[tokio::test]
async fn test_delete_pack_action_recovers_corrupted_and_oversized_without_affecting_healthy_pack() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    std::fs::create_dir_all(&storage_dir).unwrap();
    let (input_uc, _) = build_services(storage_dir.clone(), tmp.path().to_path_buf());

    let healthy_pack = input_uc
        .create_with_tags_ttl(Some("healthy-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let healthy_id = healthy_pack.id.clone();

    let malformed_id = PackId::new();
    let malformed_path = storage_dir.join(format!("{}.json", malformed_id.as_str()));
    std::fs::write(&malformed_path, "not-json").unwrap();

    let oversized_id = PackId::new();
    let oversized_pack = oversized_pack_for_limit(oversized_id.clone(), 524_288);
    let oversized_payload = serde_json::to_string(&oversized_pack).unwrap();
    assert!(
        oversized_payload.len() > 524_288,
        "oversized payload must be above default max"
    );
    let oversized_path = storage_dir.join(format!("{}.json", oversized_id.as_str()));
    std::fs::write(&oversized_path, oversized_payload).unwrap();

    assert!(
        malformed_path.exists(),
        "corrupted pack should exist before recovery"
    );
    assert!(
        oversized_path.exists(),
        "oversized pack should exist before recovery"
    );

    let healthy_before = input_uc
        .get(healthy_id.as_str())
        .await
        .expect("healthy pack should remain readable");
    assert_eq!(healthy_before.id, healthy_id);

    assert!(
        input_uc
            .delete_pack_file(malformed_id.as_str())
            .await
            .unwrap(),
        "delete_pack should remove malformed pack"
    );
    assert!(
        input_uc
            .delete_pack_file(oversized_id.as_str())
            .await
            .unwrap(),
        "delete_pack should remove oversized pack"
    );
    assert!(
        !malformed_path.exists(),
        "malformed pack should be removed by operator recovery action"
    );
    assert!(
        !oversized_path.exists(),
        "oversized pack should be removed by operator recovery action"
    );

    let healthy_after = input_uc.get(healthy_id.as_str()).await.unwrap();
    assert_eq!(healthy_after.id, healthy_id);
    let listed = input_uc.list(None, None, None, None).await.unwrap();
    assert_eq!(
        listed.len(),
        1,
        "healthy pack should be only remaining pack after recovery cleanup"
    );
    assert_eq!(listed[0].id, healthy_id);
}

#[tokio::test]
async fn test_schema_mismatch_returns_migration_required() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    std::fs::create_dir_all(&storage_dir).unwrap();
    std::fs::write(
        storage_dir.join("pk_aaaaaaaa.json"),
        r#"{"schema_version":1,"id":"pk_aaaaaaaa","name":"old-pack","title":null,"brief":null,"status":"draft","tags":[],"sections":[],"revision":1,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","expires_at":"2099-01-01T00:00:00Z"}"#,
    )
    .unwrap();

    let (input_uc, _) = build_services(storage_dir, tmp.path().to_path_buf());
    let listed = input_uc.list(None, None, None, None).await;
    assert!(matches!(listed, Err(DomainError::MigrationRequired(_))));
}

#[tokio::test]
async fn test_expired_pack_is_not_visible_immediately_in_list() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let (input_uc, _) = build_services(storage_dir.clone(), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("ttl-hidden".into()), None, None, None, 30)
        .await
        .unwrap();
    let pack_id = pack.id.as_str().to_string();
    let path = storage_dir.join(format!("{}.json", pack_id));

    let raw = std::fs::read_to_string(&path).unwrap();
    let mut data: serde_json::Value = serde_json::from_str(&raw).unwrap();
    data["expires_at"] = serde_json::Value::String("2000-01-01T00:00:00Z".into());
    let new_json = serde_json::to_string(&data).unwrap();
    std::fs::write(path, new_json).unwrap();

    let listed = input_uc.list(None, None, None, None).await.unwrap();
    assert!(
        listed.iter().all(|p| p.id.as_str() != pack_id),
        "expired pack must be invisible in list"
    );

    let expired_only = input_uc
        .list_with_freshness(None, None, None, None, Some(FreshnessState::Expired))
        .await
        .unwrap();
    assert!(
        expired_only.iter().any(|p| p.id.as_str() == pack_id),
        "freshness filter must surface expired packs intentionally"
    );
}

#[tokio::test]
async fn test_get_rendered_exposes_freshness_state_and_warning_when_expiring_soon() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("sample.rs"), "fn sample() {}\n").unwrap();

    let (input_uc, output_uc) = build_services(storage_dir.clone(), tmp.path().to_path_buf());

    let pack = input_uc
        .create_with_tags_ttl(Some("expiring-pack".into()), None, None, None, 30)
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let path = storage_dir.join(format!("{}.json", id));
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["expires_at"] = serde_json::Value::String(
        (Utc::now() + Duration::seconds(FreshnessState::EXPIRING_SOON_THRESHOLD_SECONDS))
            .to_rfc3339(),
    );
    std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();

    let rendered = output_uc.get_rendered(&id, None).await.unwrap();
    assert_eq!(
        legend_value(&rendered, "freshness_state").as_deref(),
        Some("expiring_soon")
    );
    assert!(
        legend_value(&rendered, "warning")
            .as_deref()
            .is_some_and(|warning| warning.contains("expiring soon")),
        "expiring packs must include concise warning text"
    );
    assert!(
        legend_value(&rendered, "expires_at").is_some()
            && legend_value(&rendered, "ttl_remaining").is_some(),
        "get legend must include stable freshness metadata"
    );
}

fn legend_value(rendered: &str, key: &str) -> Option<String> {
    let prefix = format!("- {}: ", key);
    rendered
        .lines()
        .find_map(|line| line.trim().strip_prefix(&prefix).map(str::to_string))
}

fn extract_next_page_token(rendered: &str) -> Option<String> {
    let raw = legend_value(rendered, "next_page_token")?;
    if raw == "null" {
        None
    } else {
        Some(raw)
    }
}

fn rendered_ref_keys(rendered: &str) -> Vec<String> {
    rendered
        .lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("#### ")?;
            let (key, _) = rest.split_once(" [")?;
            if key.starts_with("ref-") {
                Some(key.to_string())
            } else {
                None
            }
        })
        .collect()
}

async fn seed_pack_with_refs(
    input_uc: &InputUseCases,
    source_root: &std::path::Path,
    name: &str,
    refs_count: usize,
) -> String {
    let file = source_root.join("paging.rs");
    let mut content = String::new();
    for i in 1..=refs_count {
        content.push_str(&format!(
            "fn item_{i:02}() {{ let token = \"TOKEN_{i:02}\"; }}\n"
        ));
    }
    std::fs::write(&file, content).unwrap();

    let pack = input_uc
        .create_with_tags_ttl(
            Some(name.into()),
            Some("Paging pack".into()),
            None,
            None,
            30,
        )
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let mut revision = pack.revision;

    let pack = input_uc
        .upsert_section_checked(
            &id,
            "sec-one",
            "Section One".into(),
            Some("chunked refs".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    for i in 1..=refs_count {
        let pack = input_uc
            .upsert_ref_checked(
                &id,
                UpsertRefRequest {
                    section_key: "sec-one".into(),
                    ref_key: format!("ref-{i:02}"),
                    path: "src/paging.rs".into(),
                    line_start: i,
                    line_end: i,
                    title: Some(format!("Ref {i:02}")),
                    why: Some(format!("token {i:02}")),
                    group: None,
                },
                revision,
            )
            .await
            .unwrap();
        revision = pack.revision;
    }

    id
}

#[tokio::test]
async fn test_output_read_reviewer_profile_preserves_full_evidence() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();
    std::fs::write(source_root.join("sample.rs"), "line1\nline2\n").unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());
    let pack = input_uc
        .create_with_tags_ttl(
            Some("reviewer-pack".into()),
            Some("Reviewer Pack".into()),
            None,
            None,
            30,
        )
        .await
        .unwrap();
    let id = pack.id.as_str().to_string();
    let pack = input_uc
        .upsert_section_checked(&id, "sec", "Section".into(), None, None, pack.revision)
        .await
        .unwrap();
    input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "sec".into(),
                ref_key: "ref-01".into(),
                path: "src/sample.rs".into(),
                line_start: 1,
                line_end: 2,
                title: Some("ref".into()),
                why: None,
                group: None,
            },
            pack.revision,
        )
        .await
        .unwrap();

    let rendered = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                profile: Some(OutputProfile::Reviewer),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        legend_value(&rendered, "profile").as_deref(),
        Some("reviewer")
    );
    assert!(rendered.contains("```rust"));
    assert!(!rendered.contains("- paging: active"));
    assert!(!rendered.contains("- next_page_token:"));
}

#[tokio::test]
async fn test_output_read_orchestrator_default_is_compact_and_bounded() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());
    let id = seed_pack_with_refs(&input_uc, &source_root, "orchestrator-pack", 8).await;

    let rendered = output_uc
        .get_rendered_with_request(&id, OutputReadRequest::default())
        .await
        .unwrap();

    assert_eq!(
        legend_value(&rendered, "profile").as_deref(),
        Some("orchestrator")
    );
    assert_eq!(legend_value(&rendered, "mode").as_deref(), Some("compact"));
    assert_eq!(legend_value(&rendered, "paging").as_deref(), Some("active"));
    assert_eq!(legend_value(&rendered, "limit").as_deref(), Some("6"));
    assert_eq!(legend_value(&rendered, "has_more").as_deref(), Some("true"));
    assert!(legend_value(&rendered, "next_page_token").is_some());
    assert!(rendered.contains("## Handoff summary [handoff]"));
    assert!(!rendered.contains("```rust"));
    assert_eq!(
        rendered_ref_keys(&rendered).len(),
        6,
        "orchestrator profile should use bounded compact default"
    );
}

#[tokio::test]
async fn test_output_read_executor_profile_is_actionable_compact() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());
    let id = seed_pack_with_refs(&input_uc, &source_root, "executor-pack", 13).await;

    let rendered = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                profile: Some(OutputProfile::Executor),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        legend_value(&rendered, "profile").as_deref(),
        Some("executor")
    );
    assert_eq!(legend_value(&rendered, "mode").as_deref(), Some("compact"));
    assert_eq!(legend_value(&rendered, "limit").as_deref(), Some("12"));
    assert_eq!(legend_value(&rendered, "has_more").as_deref(), Some("true"));
    assert!(rendered.contains("- path: src/paging.rs"));
    assert!(!rendered.contains("```rust"));
}

#[tokio::test]
async fn test_output_read_page_token_paging_is_deterministic_and_validated() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());
    let id = seed_pack_with_refs(&input_uc, &source_root, "paging-pack", 4).await;

    let page1 = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                profile: Some(OutputProfile::Orchestrator),
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(legend_value(&page1, "has_more").as_deref(), Some("true"));
    let page_token = extract_next_page_token(&page1).expect("next page_token expected");

    let page2 = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                page_token: Some(page_token.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(legend_value(&page2, "has_more").as_deref(), Some("false"));
    assert_eq!(
        legend_value(&page2, "next_page_token").as_deref(),
        Some("null")
    );

    let mut merged = rendered_ref_keys(&page1);
    merged.extend(rendered_ref_keys(&page2));
    assert_eq!(
        merged,
        vec!["ref-01", "ref-02", "ref-03", "ref-04"],
        "page_token traversal must be complete without duplicates/gaps"
    );

    let mismatch = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                page_token: Some(page_token.clone()),
                profile: Some(OutputProfile::Reviewer),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(mismatch, DomainError::InvalidData(msg) if msg.contains("invalid_page_token") && msg.contains("fingerprint"))
    );

    let snapshot = input_uc.get(&id).await.unwrap();
    input_uc
        .upsert_section_checked(
            &id,
            "sec-extra",
            "Extra".into(),
            None,
            None,
            snapshot.revision,
        )
        .await
        .unwrap();

    let stale = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                page_token: Some(page_token),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(
        matches!(stale, DomainError::InvalidData(msg) if msg.contains("invalid_page_token") && msg.contains("revision"))
    );
}

#[tokio::test]
async fn test_output_read_contains_filter() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    let source_root = tmp.path().join("src");
    std::fs::create_dir_all(&source_root).unwrap();

    let (input_uc, output_uc) = build_services(storage_dir, tmp.path().to_path_buf());
    let id = seed_pack_with_refs(&input_uc, &source_root, "contains-pack", 3).await;

    let filtered = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                profile: Some(OutputProfile::Orchestrator),
                contains: Some("token_02".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rendered_ref_keys(&filtered), vec!["ref-02"]);
    assert_eq!(
        legend_value(&filtered, "contains").as_deref(),
        Some("token_02")
    );

    let no_match = output_uc
        .get_rendered_with_request(
            &id,
            OutputReadRequest {
                profile: Some(OutputProfile::Orchestrator),
                contains: Some("NO_SUCH_TOKEN".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(no_match.contains("_No chunks matched current filters._"));
}

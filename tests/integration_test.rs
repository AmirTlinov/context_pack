use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

use mcp_context_pack::{
    adapters::{code_excerpt_fs::CodeExcerptFsAdapter, storage_json::JsonStorageAdapter},
    app::{
        input_usecases::{InputUseCases, TouchTtlMode, UpsertRefRequest},
        output_usecases::OutputUseCases,
    },
    domain::errors::DomainError,
    domain::types::Status,
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

    // Add section
    let pack = input_uc
        .upsert_section_checked(
            &pack_id,
            "sec-one",
            "Section One".into(),
            Some("desc".into()),
            None,
            revision,
        )
        .await
        .unwrap();
    revision = pack.revision;

    // Add ref
    let pack = input_uc
        .upsert_ref_checked(
            &pack_id,
            UpsertRefRequest {
                section_key: "sec-one".into(),
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

    // Finalize
    input_uc
        .set_status_checked(&pack_id, Status::Finalized, revision)
        .await
        .unwrap();

    // Render output
    let rendered = output_uc.get_rendered(&pack_id, None).await.unwrap();

    assert!(rendered.contains("[LEGEND]"), "must have LEGEND section");
    assert!(rendered.contains("[CONTENT]"), "must have CONTENT section");
    assert!(rendered.contains("# Context pack: My Test Pack"));
    assert!(rendered.contains("## Section One"));
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
    assert!(res.is_err(), "cannot finalize empty pack");
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
        res.is_err(),
        "cannot finalize pack with empty sections only"
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
        .upsert_section_checked(&id, "sec-one", "S".into(), None, None, pack.revision)
        .await
        .unwrap();
    // Ref points to lines 10-20 (past end of 2-line file)
    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "sec-one".into(),
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
    let finalize = input_uc
        .set_status_checked(&id, Status::Finalized, pack.revision)
        .await;
    assert!(
        finalize.is_err(),
        "finalize must fail-closed when refs are stale"
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
    assert!(matches!(
        stale,
        Err(DomainError::RevisionConflict {
            expected,
            actual
        }) if expected == revision && actual == revision + 1
    ));
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
        .upsert_section_checked(&id, "sec-one", "S".into(), None, None, pack.revision)
        .await
        .unwrap();
    let pack = input_uc
        .upsert_ref_checked(
            &id,
            UpsertRefRequest {
                section_key: "sec-one".into(),
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

    let finalize = input_uc
        .set_status_checked(&id, Status::Finalized, pack.revision)
        .await;
    assert!(
        finalize.is_err(),
        "finalize must fail when line_end is stale"
    );
}

#[tokio::test]
async fn test_malformed_pack_file_fails_closed() {
    let tmp = tempdir().unwrap();
    let storage_dir = tmp.path().join("packs");
    std::fs::create_dir_all(&storage_dir).unwrap();
    std::fs::write(storage_dir.join("pk_aaaaaaaa.json"), "not-json").unwrap();

    let (input_uc, _) = build_services(storage_dir, tmp.path().to_path_buf());
    let listed = input_uc.list(None, None, None, None).await;
    assert!(matches!(listed, Err(DomainError::Deserialize(_))));
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
}

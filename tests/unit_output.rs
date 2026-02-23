//! Unit tests for OutputUseCases using in-memory fakes.
//! No filesystem or network access — all dependencies are faked.

use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc, sync::Mutex};

use mcp_context_pack::{
    app::{
        output_usecases::OutputUseCases,
        ports::{CodeExcerptPort, ListFilter, PackRepositoryPort, Snippet},
    },
    domain::{
        errors::{DomainError, Result},
        models::Pack,
        types::{LineRange, PackId, PackName, RelativePath, Status},
    },
};

// ── FakePackRepo ─────────────────────────────────────────────────────────────

struct FakePackRepo(Mutex<HashMap<String, Pack>>);

impl FakePackRepo {
    fn with(packs: Vec<Pack>) -> Arc<Self> {
        let map = packs
            .into_iter()
            .map(|p| (p.id.as_str().to_string(), p))
            .collect();
        Arc::new(Self(Mutex::new(map)))
    }
}

#[async_trait]
impl PackRepositoryPort for FakePackRepo {
    async fn create_new(&self, pack: &Pack) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(pack.id.as_str().to_string(), pack.clone());
        Ok(())
    }

    async fn save_with_expected_revision(&self, pack: &Pack, _expected: u64) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(pack.id.as_str().to_string(), pack.clone());
        Ok(())
    }

    async fn get_by_id(&self, id: &PackId) -> Result<Option<Pack>> {
        Ok(self.0.lock().unwrap().get(id.as_str()).cloned())
    }

    async fn get_by_name(&self, name: &PackName) -> Result<Option<Pack>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .values()
            .find(|p| p.name.as_ref().map(|n| n.as_str()) == Some(name.as_str()))
            .cloned())
    }

    async fn list_packs(&self, _filter: ListFilter) -> Result<Vec<Pack>> {
        Ok(self.0.lock().unwrap().values().cloned().collect())
    }

    async fn delete_pack_file(&self, id: &PackId) -> Result<bool> {
        Ok(self.0.lock().unwrap().remove(id.as_str()).is_some())
    }

    async fn purge_expired(&self) -> Result<()> {
        Ok(())
    }
}

// ── FakeExcerptPort ──────────────────────────────────────────────────────────

struct FakeExcerptPort(HashMap<String, String>); // path -> pre-rendered body

impl FakeExcerptPort {
    fn with(entries: Vec<(&str, &str)>) -> Arc<Self> {
        Arc::new(Self(
            entries
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ))
    }

    /// Returns StaleRef for every path.
    fn stale() -> Arc<Self> {
        Arc::new(Self(HashMap::new()))
    }
}

#[async_trait]
impl CodeExcerptPort for FakeExcerptPort {
    async fn read_lines(&self, path: &RelativePath, range: LineRange) -> Result<Snippet> {
        match self.0.get(path.as_str()) {
            None => Err(DomainError::StaleRef(format!("stale: {}", path.as_str()))),
            Some(body) => Ok(Snippet {
                path: path.as_str().to_string(),
                line_start: range.start,
                line_end: range.end,
                body: body.clone(),
                total_lines: range.end,
            }),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_output(packs: Vec<Pack>, excerpt: Arc<dyn CodeExcerptPort>) -> OutputUseCases {
    OutputUseCases::new(FakePackRepo::with(packs), excerpt)
}

fn simple_pack() -> Pack {
    Pack::new(PackId::new(), None)
}

fn named_pack(name: &str) -> Pack {
    Pack::new(PackId::new(), Some(PackName::new(name).unwrap()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// [LEGEND] section always contains the pack id and status.
#[tokio::test]
async fn test_render_legend_contains_id_and_status() {
    let pack = simple_pack();
    let id_str = pack.id.as_str().to_string();
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(rendered.contains("[LEGEND]"), "missing [LEGEND]");
    assert!(rendered.contains(&id_str), "id missing from legend");
    assert!(rendered.contains("draft"), "status missing from legend");
}

/// Name and brief appear in the legend when set.
#[tokio::test]
async fn test_render_with_name_and_brief() {
    let mut pack = named_pack("my-feature");
    pack.brief = Some("brief description".to_string());
    let id_str = pack.id.as_str().to_string();
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(rendered.contains("my-feature"), "name missing");
    assert!(rendered.contains("brief description"), "brief missing");
}

/// Tags line appears in the legend when tags are set.
#[tokio::test]
async fn test_render_with_tags() {
    let mut pack = simple_pack();
    pack.tags = vec!["rust".to_string(), "backend".to_string()];
    let id_str = pack.id.as_str().to_string();
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(rendered.contains("rust"), "tag 'rust' missing");
    assert!(rendered.contains("backend"), "tag 'backend' missing");
}

/// [CONTENT] contains section title and a code block when a ref is present.
#[tokio::test]
async fn test_render_with_section_and_ref() {
    use mcp_context_pack::domain::models::{CodeRef, Section};
    use mcp_context_pack::domain::types::{RefKey, SectionKey};

    let mut pack = simple_pack();
    let section_key = SectionKey::new("main-section").unwrap();
    let ref_key = RefKey::new("my-ref").unwrap();
    let path = RelativePath::new("src/lib.rs").unwrap();
    let lines = LineRange::new(1, 3).unwrap();

    let code_ref = CodeRef {
        key: ref_key,
        path: path.clone(),
        lines,
        title: Some("My ref".to_string()),
        why: None,
        group: None,
    };
    let section = Section {
        key: section_key,
        title: "Main Section".to_string(),
        description: None,
        refs: vec![code_ref],
        diagrams: vec![],
    };
    pack.sections = vec![section];

    let id_str = pack.id.as_str().to_string();
    let uc = make_output(
        vec![pack],
        FakeExcerptPort::with(vec![("src/lib.rs", "   1: fn main() {}")]),
    );
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(rendered.contains("[CONTENT]"), "missing [CONTENT]");
    assert!(rendered.contains("Main Section"), "section title missing");
    assert!(rendered.contains("fn main()"), "code excerpt missing");
}

/// Stale ref renders as "> stale ref:" warning line.
#[tokio::test]
async fn test_render_stale_ref_shown_as_warning() {
    use mcp_context_pack::domain::models::{CodeRef, Section};
    use mcp_context_pack::domain::types::{RefKey, SectionKey};

    let mut pack = simple_pack();
    let section = Section {
        key: SectionKey::new("s1").unwrap(),
        title: "Section".to_string(),
        description: None,
        refs: vec![CodeRef {
            key: RefKey::new("r1").unwrap(),
            path: RelativePath::new("missing.rs").unwrap(),
            lines: LineRange::new(1, 2).unwrap(),
            title: None,
            why: None,
            group: None,
        }],
        diagrams: vec![],
    };
    pack.sections = vec![section];

    let id_str = pack.id.as_str().to_string();
    // FakeExcerptPort::stale() returns StaleRef for every path
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(
        rendered.contains("> stale ref:"),
        "expected stale ref warning, got:\n{rendered}"
    );
}

/// Mermaid diagram renders as ```mermaid block.
#[tokio::test]
async fn test_render_with_diagram_block() {
    use mcp_context_pack::domain::models::{Diagram, Section};
    use mcp_context_pack::domain::types::{DiagramKey, SectionKey};

    let mut pack = simple_pack();
    let section = Section {
        key: SectionKey::new("s1").unwrap(),
        title: "Section".to_string(),
        description: None,
        refs: vec![],
        diagrams: vec![Diagram {
            key: DiagramKey::new("d1").unwrap(),
            title: "My Diagram".to_string(),
            mermaid: "graph TD; A-->B".to_string(),
            why: None,
        }],
    };
    pack.sections = vec![section];

    let id_str = pack.id.as_str().to_string();
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(
        rendered.contains("```mermaid"),
        "expected mermaid block, got:\n{rendered}"
    );
    assert!(rendered.contains("graph TD"), "diagram content missing");
}

/// get_rendered with status_filter=Finalized on a Draft pack returns InvalidState.
#[tokio::test]
async fn test_get_rendered_status_filter_mismatch() {
    let pack = simple_pack(); // status = Draft
    let id_str = pack.id.as_str().to_string();
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    let err = uc
        .get_rendered(&id_str, Some(Status::Finalized))
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::InvalidState(_)),
        "expected InvalidState, got: {:?}",
        err
    );
}

/// get_rendered with unknown id returns NotFound.
#[tokio::test]
async fn test_get_rendered_not_found_id() {
    let uc = make_output(vec![], FakeExcerptPort::stale());
    // Use a valid PackId format that doesn't exist in the repo
    let fake_id = PackId::new().as_str().to_string();
    let err = uc.get_rendered(&fake_id, None).await.unwrap_err();
    assert!(
        matches!(err, DomainError::NotFound(_)),
        "expected NotFound, got: {:?}",
        err
    );
}

/// get_rendered with empty identifier returns InvalidData.
#[tokio::test]
async fn test_get_rendered_empty_identifier() {
    let uc = make_output(vec![], FakeExcerptPort::stale());
    let err = uc.get_rendered("", None).await.unwrap_err();
    assert!(
        matches!(err, DomainError::InvalidData(_)),
        "expected InvalidData for empty identifier, got: {:?}",
        err
    );
}

/// list_filtered returns all seeded packs.
#[tokio::test]
async fn test_list_filtered_returns_packs() {
    let p1 = simple_pack();
    let p2 = simple_pack();
    let ids: Vec<String> = vec![p1.id.as_str().to_string(), p2.id.as_str().to_string()];
    let uc = make_output(vec![p1, p2], FakeExcerptPort::stale());
    let result = uc.list_filtered(None, None, None, None).await.unwrap();
    assert_eq!(result.len(), 2, "expected 2 packs");
    for pack in &result {
        assert!(ids.contains(&pack.id.as_str().to_string()));
    }
}

/// .rs file path produces ```rust language tag in rendered output.
#[tokio::test]
async fn test_lang_detection_rust_extension() {
    use mcp_context_pack::domain::models::{CodeRef, Section};
    use mcp_context_pack::domain::types::{RefKey, SectionKey};

    let mut pack = simple_pack();
    pack.sections = vec![Section {
        key: SectionKey::new("s1").unwrap(),
        title: "S".to_string(),
        description: None,
        refs: vec![CodeRef {
            key: RefKey::new("r1").unwrap(),
            path: RelativePath::new("src/main.rs").unwrap(),
            lines: LineRange::new(1, 1).unwrap(),
            title: None,
            why: None,
            group: None,
        }],
        diagrams: vec![],
    }];
    let id_str = pack.id.as_str().to_string();
    let uc = make_output(
        vec![pack],
        FakeExcerptPort::with(vec![("src/main.rs", "   1: fn main() {}")]),
    );
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    assert!(
        rendered.contains("```rust"),
        "expected ```rust language tag, got:\n{rendered}"
    );
}

/// Unknown file extension produces ``` with no language tag.
#[tokio::test]
async fn test_lang_detection_unknown_extension() {
    use mcp_context_pack::domain::models::{CodeRef, Section};
    use mcp_context_pack::domain::types::{RefKey, SectionKey};

    let mut pack = simple_pack();
    pack.sections = vec![Section {
        key: SectionKey::new("s1").unwrap(),
        title: "S".to_string(),
        description: None,
        refs: vec![CodeRef {
            key: RefKey::new("r1").unwrap(),
            path: RelativePath::new("data/file.xyz").unwrap(),
            lines: LineRange::new(1, 1).unwrap(),
            title: None,
            why: None,
            group: None,
        }],
        diagrams: vec![],
    }];
    let id_str = pack.id.as_str().to_string();
    let uc = make_output(
        vec![pack],
        FakeExcerptPort::with(vec![("data/file.xyz", "   1: some data")]),
    );
    let rendered = uc.get_rendered(&id_str, None).await.unwrap();
    // Should contain ``` but NOT ```rust or any other lang tag
    assert!(rendered.contains("```\n"), "expected empty lang tag ```\\n");
    assert!(
        !rendered.contains("```rust"),
        "should not have rust lang tag for .xyz"
    );
}

/// Named pack can be resolved by name identifier.
#[tokio::test]
async fn test_resolve_by_name() {
    let pack = named_pack("resolve-by-name");
    let uc = make_output(vec![pack], FakeExcerptPort::stale());
    // Resolve using the pack name (not id)
    let rendered = uc.get_rendered("resolve-by-name", None).await.unwrap();
    assert!(
        rendered.contains("[LEGEND]"),
        "missing [LEGEND] when resolved by name"
    );
    assert!(
        rendered.contains("resolve-by-name"),
        "name should appear in rendered output"
    );
}

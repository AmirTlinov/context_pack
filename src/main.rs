use std::path::PathBuf;
use std::sync::Arc;

fn source_root_from_env_or_cwd() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let raw = match std::env::var("CONTEXT_PACK_SOURCE_ROOT") {
        Ok(v) => v,
        Err(_) => return cwd,
    };

    let value = raw.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("cwd")
        || value.eq_ignore_ascii_case("session_cwd")
        || value.eq_ignore_ascii_case("__SESSION_CWD__")
        || value == "."
    {
        cwd
    } else {
        PathBuf::from(value)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env_filter = if std::env::var("CONTEXT_PACK_LOG").is_ok() {
        tracing_subscriber::EnvFilter::from_env("CONTEXT_PACK_LOG")
    } else {
        tracing_subscriber::EnvFilter::new("mcp_context_pack=info")
    };

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr) // log to stderr so stdout stays clean for MCP
        .with_env_filter(env_filter)
        .init();

    let source_root = source_root_from_env_or_cwd();

    let storage_root = std::env::var("CONTEXT_PACK_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".agents").join("mcp").join("context_pack"));

    let storage_dir = storage_root.join("packs");

    tracing::info!("storage dir: {}", storage_dir.display());
    tracing::info!("source root: {}", source_root.display());

    let storage =
        Arc::new(mcp_context_pack::adapters::storage_json::JsonStorageAdapter::new(storage_dir));
    let excerpts = Arc::new(
        mcp_context_pack::adapters::code_excerpt_fs::CodeExcerptFsAdapter::new(source_root)
            .map_err(anyhow::Error::new)?,
    );

    let input_uc = Arc::new(mcp_context_pack::app::input_usecases::InputUseCases::new(
        storage.clone(),
        excerpts.clone(),
    ));
    let output_uc = Arc::new(mcp_context_pack::app::output_usecases::OutputUseCases::new(
        storage.clone(),
        excerpts.clone(),
    ));

    mcp_context_pack::adapters::mcp_stdio::start_mcp_server(input_uc, output_uc).await?;

    Ok(())
}

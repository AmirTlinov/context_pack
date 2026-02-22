# Release guide

This repository ships prebuilt binaries via GitHub Releases.

## Trigger a release

1. Ensure `main` is green (`ci` workflow passes).
2. Create and push a semantic tag:

```bash
git tag v0.1.0
git push origin v0.1.0
```

3. GitHub Actions workflow `.github/workflows/release.yml` will:
   - build binaries for supported targets,
   - package archives,
   - generate `checksums.sha256`,
   - generate Homebrew formula and Scoop manifest,
   - publish assets to the tag release.

Install scripts (`scripts/install.sh`, `scripts/install.ps1`) validate archives against
the published `checksums.sha256`.

## Published assets

- `mcp-context-pack-x86_64-unknown-linux-gnu.tar.gz`
- `mcp-context-pack-aarch64-unknown-linux-gnu.tar.gz`
- `mcp-context-pack-x86_64-apple-darwin.tar.gz`
- `mcp-context-pack-aarch64-apple-darwin.tar.gz`
- `mcp-context-pack-x86_64-pc-windows-msvc.zip`
- `checksums.sha256`
- `mcp-context-pack.rb` (Homebrew formula)
- `mcp-context-pack.json` (Scoop manifest)

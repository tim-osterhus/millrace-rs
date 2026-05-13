# Millrace Workspace Baselines And Upgrades

Rust workspaces use an explicit managed baseline under
`<workspace>/millrace-agents/`.

## Initialization

```bash
millrace init --workspace <workspace>
```

Initialization deploys package-managed entrypoints, skills, modes, graphs,
loops, registry files, runtime config, status/state files, history/outline
files, and `state/baseline_manifest.json`.

## Baseline Manifest

The baseline manifest records the package-deployed managed asset set with
relative paths, asset families, and content hashes. Existing workspaces do not
silently adopt new package assets when the crate changes; operators refresh
them explicitly through upgrade preview/apply.

## Upgrade

```bash
millrace upgrade --workspace <workspace>
millrace upgrade --workspace <workspace> --apply
```

Preview classifies managed files as unchanged, safe package updates, local-only
modifications, already converged, missing, localized removed, or conflicts.
Apply writes only safe managed updates, restores missing managed files, and
refuses unresolved conflicts.

## v0.18.3 Package Evidence

Rust `0.3.3` package evidence covers the Librarian entrypoint, stage-kind
registry, `librarian-core` skill, learning graph/loop assets, learning mode
trigger assets, shipped skill lint assets, runtime docs, parity fixtures, and
release evidence. It deliberately excludes live runtime workspace artifacts
such as `millrace-agents/**`, `ideas/**`, and `target/**`, plus generated
Python cache artifacts under `__pycache__/` and `*.pyc`/`*.pyo` paths, from the
crate package include rules.

## v0.18.4 Package Evidence

Rust `0.3.4` package evidence covers runner failure classifier metadata,
blocked-task diagnostics, audited `queue retry-blocked` behavior,
`[auto_recovery]` config/status handling, daemon blocked-dependency recovery,
runtime docs, parity fixtures, and release evidence. It preserves the same
package boundary: live runtime workspace artifacts, the optional Python
`packages/millrace-web` package, `target/**`, and generated Python cache
artifacts remain excluded from the crate package include rules.

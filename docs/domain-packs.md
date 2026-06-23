# Domain Packs: Registry, Lifecycle, and Authoring

Domain packs add optional skills, workflows, personas, and tools without making
the default agent007 catalog project-specific. Packs are explicit, versioned,
integrity-pinned, reversible overlays; they do not copy files into a user's
editable catalog directories.

## Mental model

```text
                       OFFICIAL GITHUB REPOSITORY
                       ==========================

  packs/<id>/ source        registry/v1/index.json       release assets
  +------------------+      +---------------------+      +-------------+
  | pack.toml        | CI   | versions            | URL  | .a7bundle   |
  | skills/          |----->| compatibility       |----->| manifest    |
  | workflows/       |      | size + SHA-256      |      | checksums   |
  | personas/ tools/ |      +---------------------+      +-------------+
  +------------------+                 |
                                       | search / inspect / install
                                       v
                         ONE AGENT007 INSTALL SCOPE
                         ==========================

  ~/.agent007/packs/                  <project>/.agent007/packs/
  +-- cache/registry.json             +-- cache/registry.json
  +-- lock.json                       +-- lock.json
  `-- <id>/<version>/                 `-- <id>/<version>/
      +-- pack.toml                       +-- pack.toml
      +-- artifact.a7bundle               +-- artifact.a7bundle
      `-- skills/workflows/...            `-- skills/workflows/...
```

An enabled lockfile entry adds that version's component directories to catalog
lookup. Disabling changes the lockfile only. Uninstall removes versions owned by
that pack only.

## User commands

```bash
# Discovery and trust inspection
agent007 pack search finance
agent007 pack info example-hello

# Global lifecycle (default scope)
agent007 pack install example-hello
agent007 pack list
agent007 pack disable example-hello
agent007 pack enable example-hello
agent007 pack update example-hello
agent007 pack rollback example-hello
agent007 pack uninstall example-hello --yes

# Project-local lifecycle
agent007 pack install example-hello --scope project
agent007 pack list --scope project

# Cache and registry controls
agent007 pack search --offline
agent007 pack verify-registry --registry registry/v1/index.json --refresh
```

Set `AGENT007_PACK_REGISTRY` to a reviewed alternate index. Passing `--registry`
on discovery, install, update, or verification commands overrides it for that
operation. Third-party registries are not automatically trusted.

The Hub exposes the same global lifecycle at `agent007 hub --port 8006`. Open
the **Domain packs** rail item to inspect declared contents, dependencies,
network access, external actions, and approval gates before installation. Hub
actions are labeled **global scope**; project-only packs remain available through
`agent007 pack ... --scope project` so a Hub action cannot silently target the
wrong registered project.

## Runtime visibility

Enabled pack contents are part of the normal catalog, not a Hub-only inventory:

```text
enabled pack lock
      |
      +--> CLI skill / workflow / persona lookup
      +--> MCP list and generic run tools
      +--> hosted workflows and agent skill providers
      +--> dashboard catalog APIs and tool resolution
      +--> Hub Global skills / workflows / personas (read-only pack badge)
      `--> RAG warmup indexing for pack-provided skill templates
```

For example, an enabled skill is visible to `agent007 skill list` and can be run
through `agent007_skill_run` without copying it into `~/.agent007/skills`.
Pack-provided assets appear read-only in both dashboards because changing
installed bytes would invalidate the verified artifact. The normal dashboard can
save a project/global override without modifying the pack. Use **Domain packs**
to disable, update, rollback, or uninstall the original package.

MCP clients cache the server's advertised individual dynamic tool names. The
generic list/run tools read the catalog on every call, but a newly installed
pack's dedicated `agent007_skill_<name>` tool becomes visible to the client after
the agent007 MCP process is restarted.

## Precedence

Higher rows win when two assets have the same identity:

```text
1. manually maintained project asset
2. enabled project pack asset
3. manually maintained global asset
4. enabled global pack asset
5. built-in fallback, where that asset type has one
```

This lets a project override a global pack without modifying or forking the
installed package. Pack-owned files should be treated as read-only; author a
manual override or release a new pack version instead.

## Verification and safety

Before activation, agent007 performs this chain:

```text
registry schema + IDs + semver
              |
              v
manifest bytes -- SHA-256 --> TOML schema + identity + dependencies
              |
              v
artifact bytes -- size + SHA-256 --> .a7bundle entry SHA-256 checks
              |
              v
staging directory -- successful extraction --> atomic lockfile activation
```

- Registry downloads are capped at 5 MiB, manifests at 1 MiB, and artifacts at
  100 MiB.
- `.a7bundle` v1 is text-only. Symlinks and traversal paths are rejected.
- Packs declaring `external_actions = true` are rejected by default. A reviewed
  CLI install or update must add `--allow-external-actions`. The Hub deliberately
  does not grant that approval.
- `network` and `approval_required` are inspectable policy declarations. They do
  not create an operating-system sandbox by themselves; runtime tools and zones
  remain responsible for enforcement.
- Integrity pinning is implemented now. Artifact signing/transparency-log
  verification is a documented follow-up.

If the network is unavailable, fresh cached registry metadata can be used
automatically. `--offline` requires an existing cache and never contacts the
registry.

## Authoring a pack

Create `packs/<pack-id>/pack.toml` and component directories:

```toml
schema_version = 1

[pack]
id = "my-domain"
name = "My Domain"
version = "1.0.0"
description = "Focused optional capability"
license = "Apache-2.0"
authors = ["Your Name"]

[contents]
skills = ["/my-domain-research"]
workflows = ["my-domain-analysis"]
personas = ["MyDomainAnalyst"]
tools = []

[permissions]
network = true
external_actions = false
approval_required = ["publish-report"]

[dependencies]
packs = []
# packs = [{ id = "shared-evidence", version = "^1.2" }]
```

Build and inspect a deterministic artifact:

```bash
agent007 pack build packs/my-domain \
  --output packs/my-domain/dist/my-domain-1.0.0.a7bundle --json
```

Add a `registry/v1/index.json` version entry containing the minimum compatible
agent007 version, manifest URL/hash, artifact URL/hash, exact artifact size, and
publication timestamp. Run the registry verifier before review.

## Publishing and governance

`.github/workflows/pack-registry.yml` validates lifecycle tests, rebuilds the
example artifact byte-for-byte, checks schemas, and verifies all published
manifest/artifact/entry hashes.

Maintainers publish an approved pack with the **Publish Domain Pack** workflow.
It creates an immutable `pack-<id>-v<version>` GitHub Release and uploads the
bundle, versioned manifest, and checksum file. Registry changes remain normal PRs
so source, policy declarations, hashes, and discoverability receive review.

## Recovery

- **Failed install:** no lockfile activation occurs; delete stale
  `packs/.staging/` entries if a process was forcibly killed.
- **Bad update:** run `agent007 pack rollback <id>`.
- **Temporary removal:** disable the pack; retained versions remain available.
- **Corrupt registry cache:** remove `packs/cache/registry.json` and refresh.
- **Corrupt lockfile:** pack management reports the parse error. Core startup
  ignores optional overlays rather than preventing agent007 from starting.
- **Interrupted mutation:** lifecycle changes use `packs/mutation.lock` to avoid
  lost updates between Hub and CLI processes. If the owning process was forcibly
  killed and no pack operation is running, remove that stale file and retry.
- **Dependency error:** disable/uninstall the dependent pack first. Required
  dependencies cannot be removed while still referenced.

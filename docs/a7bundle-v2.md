# .a7bundle v2 design

## Goals

- Replace JSON-only `.a7bundle` v1 with a compressed, binary-safe format.
- Preserve backward compatibility for importing existing v1 bundles.
- Make bundle contents self-describing, integrity-checked, and portable for P2P sharing.
- Track artifact versions at save/edit time instead of only at export time.

## Non-goals

- Full signing / trust framework in v2 initial rollout.
- Content-addressed storage or remote registry protocol.
- Automatic semantic version inference from prompt or code meaning.

## Problems in v1

```ascii
v1
├─ single JSON blob
├─ content stored as UTF-8 strings only
├─ weak support for nested dependency trees
├─ no safe binary payload transport
├─ large exports due to uncompressed text
└─ artifact versioning not first-class
```

## Format overview

`.a7bundle` remains the user-facing extension, but v2 changes the internal container.

```ascii
.a7bundle v2
└─ tar.zst archive
   ├─ manifest.json
   ├─ skills/
   ├─ workflows/
   ├─ personas/
   ├─ tools/
   ├─ scripts/
   └─ assets/
```

## Manifest schema

```json
{
  "bundle_format_version": 2,
  "bundle_version": 1,
  "created_at": "2026-05-13T17:30:00Z",
  "created_by": "agent007 0.3.1",
  "source_project": "agent007",
  "entries": [
    {
      "path": "skills/review-skill/SKILL.md",
      "kind": "skill",
      "artifact_id": "skill:review-skill",
      "version": "1.2.0",
      "sha256": "...",
      "size_bytes": 1823,
      "executable": false,
      "media_type": "text/markdown",
      "encoding": "utf-8",
      "dependency_group": "skill:review-skill"
    }
  ],
  "dependency_groups": [
    {
      "id": "tool:main_test.py",
      "members": [
        "tools/main_test.py",
        "tools/main_test/relatedfiles/helper.txt"
      ]
    }
  ],
  "unresolved_references": [],
  "compat": {
    "imported_from_v1": false
  }
}
```

## Entry rules

Each entry must carry:

- `path`
- `kind` (`skill`, `workflow`, `persona`, `tool`, `script`, `asset`)
- `artifact_id`
- `version`
- `sha256`
- `size_bytes`
- `executable`
- `media_type`
- `encoding`

Notes:

- `encoding` is `utf-8` for text assets and omitted or `binary` for raw assets.
- `executable` preserves tool/script executability on import.
- `dependency_group` ties flat files to sibling support trees.

## Artifact identity

Every managed artifact should have a stable identity model:

```ascii
artifact identity
├─ artifact_id
├─ name
├─ kind
├─ version
├─ created_at
├─ updated_at
├─ last_modified_by
└─ sha256
```

## Version layers

```ascii
version layers
├─ bundle_format_version
│  └─ schema/container compatibility
├─ bundle_version
│  └─ packaging revision of one exported bundle
└─ artifact version
   ├─ skill version
   ├─ workflow version
   ├─ persona version
   ├─ tool version
   └─ script/package version
```

## Version bump rules

### Artifact version

Artifact version changes on **real persisted content change**.

```ascii
create
└─ assign initial version (recommended: 1.0.0 or 0.1.0)

edit + save from dashboard
└─ bump artifact version

edit + save from hosted LLM
└─ bump artifact version

CLI save/update
└─ bump artifact version

save with no content change
└─ no version bump
```

Recommended semantics:

- patch = content/prompt/config fix
- minor = backward-compatible feature expansion
- major = breaking contract/schema/behavior change

### Import rules

```ascii
import new artifact
└─ preserve imported artifact version

import overwrite existing artifact
├─ if incoming content differs
│  └─ replace local artifact and preserve incoming version
└─ if content is identical
   └─ keep version unchanged
```

Also record local metadata:

- `imported_at`
- `imported_from`
- `imported_bundle_version`

### Export rules

```ascii
export
├─ preserve artifact versions exactly
├─ assign or increment bundle_version
└─ regenerate manifest hashes
```

## Dependency closure rules

v2 export should include:

```ascii
for selected artifact
├─ explicit file/package itself
├─ manifest-declared package members
├─ flat-tool sibling dependency directory (same stem)
├─ recursively referenced scripts/tools when resolvable
└─ dependency_groups metadata for reconstruction / preview
```

## Binary handling

v2 must support raw binary payloads.

```ascii
binary policy
├─ store as raw archive entries
├─ hash raw bytes, not decoded text
├─ preserve executable bit where relevant
└─ never force UTF-8 decoding for bundle transport
```

v1 behavior remains:

```ascii
v1
└─ reject binary/non-UTF8 clearly
```

## Compatibility plan

### Read path

- Importer must detect:
  - v1 JSON bundle
  - v2 tar.zst bundle
- v1 remains importable.
- v2 becomes the default export format.

### Write path

```ascii
phase 1
├─ keep v1 importer
└─ add v2 exporter/importer

phase 2
├─ export v2 by default
└─ keep v1 import only

phase 3
└─ optionally add v1 export only behind explicit compatibility flag
```

## UI / API implications

Dashboard should expose:

```ascii
artifact metadata
├─ current version
├─ last modified time
├─ last modified by
├─ source scope
└─ dependency summary
```

Export preview should show:

```ascii
bundle preview
├─ bundle format version
├─ bundle version
├─ artifact versions
├─ binary assets present
├─ dependency groups
└─ unresolved refs (if any)
```

## Suggested implementation order

```ascii
1. introduce artifact metadata model
2. bump versions on dashboard/LLM/CLI save paths
3. add v2 manifest structs
4. add tar.zst writer/reader
5. add raw-byte bundle asset support
6. wire export preview to show version/dependency metadata
7. keep v1 import compatibility
8. add future signing/provenance later
```

## Success criteria

- Export/import supports text and binary assets safely.
- Flat tool files can bring sibling dependency trees.
- `.a7bundle` is smaller and self-contained.
- Each saved artifact carries a meaningful version.
- Import/export preserves artifact versions correctly.
- v1 bundles still import successfully.

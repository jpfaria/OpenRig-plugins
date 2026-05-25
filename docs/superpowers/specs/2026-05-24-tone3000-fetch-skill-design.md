# tone3000 Fetch — Skill + CLI Design

- **Date:** 2026-05-24
- **Status:** approved (design), pending implementation plan
- **Issue:** TBD (to be opened at start of implementation, per repo dev-flow LAW)

## Problem

Every IR pack and NAM model in `plugins/source/{ir,nam}/` originates from
[tone3000.com](https://www.tone3000.com) — visible in each manifest's
`sources:` field. The current import flow is **fully manual**: a human
opens the tone page, clicks "Download All", unzips, renames, hand-writes
`manifest.yaml` (inferring brand / type / parameters from filenames),
then runs `qa_audit` / `pack_plugins`. This is slow, error-prone, and
hostile to discovery — there is no way to ask "what new IR / NAM packs
landed this week?" or "find me a Mesa Rectifier capture".

## Goal

Two flows, one tool:

1. **Discovery** — list/search tone3000 packs from the terminal, with
   the result annotated against the local catalogue (already-imported vs
   new).
2. **Import** — given a tone id, fetch every model + metadata, scaffold
   a `plugins/source/<kind>/<slug>/` directory in an isolated `.solvers/`
   workspace, generate a draft `manifest.yaml`, and hand control back to
   the user for review before `qa_audit` and PR.

## tone3000 API surface (verified)

The "public" REST API at `/api/v1` requires OAuth 2.0 Authorization Code
with PKCE — but the actual frontend bypasses it entirely and talks
**directly** to Supabase PostgREST at `api.tone3000.com` with an anon
JWT. That is the de-facto public API.

### Anon JWT

A single Supabase anon JWT (`role: anon`, project ref
`gzybiuopxkdxbytnojds`, expiry 2035-02-06) is embedded in every page
load:

```
eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Imd6eWJpdW9weGtkeGJ5dG5vamRzIiwicm9sZSI6ImFub24iLCJpYXQiOjE3MzgwODIxNjUsImV4cCI6MjA1MzY1ODE2NX0.Gq66BJXjtLsqP2nAGXm9Xb9PAjoeZalWUj66K4nmVSU
```

Sent as both `apikey: <jwt>` and `Authorization: Bearer <jwt>`. Same
value for everyone — no user account, no app registration.

### Endpoints (all 200 OK with the anon JWT)

| Function | Method + URL | Notes |
|---|---|---|
| Search / list | `POST /rest/v1/rpc/search_tones_a2` | Body: `{query_term, page_number, page_size, order_by, tag_names, make_names, gear_filters, is_calibrated, size_filters, usernames}`. `order_by="newest"` covers "latest" feed. |
| Filter aggregates | `POST /rest/v1/rpc/get_search_{tags,makes,usernames}_aggregate_a2` | Populates filter pickers. |
| Tone detail | `GET /rest/v1/tones?id=eq.<id>&select=*` | Title, description, gear, tags, images, user_id, links, is_public. |
| Models in a tone | `GET /rest/v1/models?tone_id=eq.<id>&select=id,name,model_url,size,position,architecture_version,created_at` | Each row is one capture (`.wav` or `.nam`). `model_url` is the direct download URL. |
| Download | `GET https://api.tone3000.com/storage/v1/object/public/models/<filename>` | **No headers required.** Public bucket. CORS `access-control-allow-origin: *`. |

`search_tones_a2` response per row (relevant fields):

```
id, title, description, tags[], makes[], gear,
model_name, models_count, a1_models_count, a2_models_count, irs_count,
created_at, updated_at, published_at, platform ("nam"|"ir"),
images[], user_id, username, avatar_url, total_count
```

### Rate-limit posture

Supabase REST endpoints have no documented throttle; observed responses
are fast and unmetered for the volume we'll generate (single-digit
requests per import, dozens per browse). A conservative client-side
governor (10 req/s, exponential backoff on 429/5xx) is sufficient.

## Components

### `tools/tone3000_fetch/` — Rust binary

Sibling to `tools/loudness_audit` and `tools/pack_plugins`. Reuses the
repo's Rust toolchain. Provides:

- An HTTP client wrapping the anon JWT, with constant-default and
  `TONE3000_ANON_KEY` env override. Fallback path: if the constant
  returns 401, fetch `https://www.tone3000.com/` and re-extract the
  current JWT from the JS bundle (one regex over the chunk that
  contains `createBrowserClient`). Cache the refreshed key under
  `~/.config/openrig/tone3000.token` so the next run skips the
  extraction.
- Typed wrappers over the four endpoints (`search_tones`, `tone_detail`,
  `tone_models`, `download_model`).
- A scaffolding layer that, given a tone id:
  - Resolves a target slug (`<brand>_<short_model>`), uniquified against
    `plugins/source/{ir,nam}/`.
  - Streams each `model_url` to `plugins/source/<kind>/<slug>/<subdir>/`
    (`ir/` for `.wav`, `captures/` for `.nam`).
  - Writes a draft `manifest.yaml` (see "Manifest inference" below).
- A presentation layer (table / JSON) that, when listing tones, marks
  each row's local status: **new** | **partial** (some captures already
  in the repo) | **imported** (same tone id appears in a manifest's
  `sources:`).

Cross-compilation parity with the rest of `tools/` (macos-universal,
linux-x86_64, linux-aarch64, windows-x86_64, windows-aarch64); no
platform-specific code.

### `.claude/skills/openrig-tone3000-fetch/` — Skill

User-facing workflow that orchestrates the binary plus the editorial
decisions Claude must own:

1. Refuse to run outside the OpenRig-plugins repo (looks for
   `tools/pack_plugins` to identify the workspace).
2. Enforce the dev-flow LAW: bring a tone in only inside a fresh
   `.solvers/issue-N/` clone, never in the user's checkout. The skill
   opens the issue (`gh issue create`) before the first download.
3. Decide the slug, brand, and parameter mapping from the inferred
   manifest — these are judgement calls (e.g. mic name normalisation,
   parameter axis naming) that change manifest output.
4. After scaffolding, run `cargo build --release -p loudness-audit --bin qa_audit`
   and `cargo run --release --bin pack_plugins`; only the user clears
   the result before push.
5. Echoes the same anti-patterns as `openrig-code-quality`: no Portuguese
   in the manifest; `sources:` must include every tone3000 URL; one
   tone-pack per issue / PR.

### Subcommands

```
tone3000 latest [--gear ir|amp|pedal|full-rig|outboard] [--platform nam|ir] [--limit N] [--json]
tone3000 search <query> [--gear …] [--tag …] [--make …] [--user …] [--limit N] [--json]
tone3000 show <tone-id>                            # full metadata + model list, with local status
tone3000 import <tone-id> [--slug <name>] [--kind ir|nam]   # full scaffold flow
tone3000 refresh-token                             # force re-extraction of the anon JWT
```

`latest` and `search` both call `search_tones_a2`; `latest` sets
`order_by="newest"` and an empty `query_term`.

## Manifest inference

A single capture row has:
- `name` (human-readable, e.g. `1970 Bassman Cabinet CTS - SM57 Upper - Cone`)
- `model_url` (filename gives `.wav` vs `.nam`)
- Tone-level: `title`, `makes[]`, `tags[]`, `gear`, `platform`

Inference produces a **draft** manifest — never authoritative. The user
edits before commit. Rules:

| Manifest field | Source | Strategy |
|---|---|---|
| `manifest_version` | constant | `1` |
| `id` | derived | `<kind>_<slug>` |
| `display_name` | tone `title` | trimmed; year/cab-size kept |
| `brand` | tone `makes[0]` | lowercased, normalised (`Fender Bassman` → `fender`); emits `# TODO: brand guessed from makes=[…]` YAML comment when `makes.len() > 1` so the user picks the right one |
| `sources` | constant | `[https://www.tone3000.com/tones/<id>]` |
| `type` | tone `gear` | tone3000 vocabulary is `ir`/`amp`/`pedal`/`full-rig`/`outboard`; OpenRig vocabulary is `cab`/`body`/`amp`/`preamp`/`gain_pedal`/`fx_pedal`/`eq`. Mapping: `ir` → `cab` (or `body` when tags contain `bass`/`acoustic`); `amp` → `amp`; `pedal` → `gain_pedal` (or `fx_pedal` when tags contain `delay`/`reverb`/`chorus`/`modulation`). `full-rig` and `outboard` cannot be expressed as a single OpenRig type — the generator emits `# TODO: <gear> not directly representable; pick type manually` and leaves the field as a placeholder. |
| `backend` | tone `platform` | `ir` → `ir`; `nam` → `nam` |
| `parameters` | parsed from each capture `name` | Token-split on `-`, classify tokens by dictionary (mic models, positions, speakers, gain numbers, voicings). Empty `values:` list if classification fails — surfaces inference miss to the user. |
| `captures` | one per model row | `values:` filled from token classification; `file:` set to the chosen subdir + sanitised filename. |
| `output_gain_db` | **omitted** | Left for `loudness_audit` / `qa_fix` to fill in the same way it does today (issue #4, #12). |

When the parameter classifier cannot resolve a token, the manifest is
emitted with an explicit `# TODO: parameter axis unresolved for <token>`
comment so the user sees the miss before running the gate.

## Integration with the existing gate

`tone3000 import` ends with:
1. The new directory under `plugins/source/<kind>/<slug>/`.
2. The draft `manifest.yaml`.
3. A prompt to the user: review the manifest, then run
   `cargo run --release --bin pack_plugins` (which itself invokes
   `qa_audit`, issue #12). The skill never runs `pack_plugins`
   automatically — that is the user's gate.

No changes to `qa_audit` thresholds or `pack_plugins` behaviour. The
audit thresholds catch the same defects whether the captures arrived
via this tool or by hand.

## File layout

```
tools/tone3000_fetch/
  Cargo.toml
  src/
    main.rs        # clap subcommand dispatch
    api.rs         # anon JWT + reqwest client + typed endpoint wrappers
    scaffold.rs    # slug, directory, manifest writer
    infer.rs       # capture-name tokeniser + parameter classifier
    local.rs       # walk plugins/source/**/manifest.yaml to detect already-imported tones
  tests/
    api_smoke.rs   # gated by TONE3000_LIVE=1; exercises each endpoint
    infer.rs       # offline fixtures for the parameter classifier
.claude/skills/openrig-tone3000-fetch/
  SKILL.md
```

## Testing

- **Offline unit tests** for `infer.rs` — dozens of capture-name strings
  from existing manifests (the `plugins/source/{ir,nam}/**/manifest.yaml`
  catalogue is the test corpus), asserting expected parameter
  classification. No network, run on every push.
- **Live smoke tests** gated by `TONE3000_LIVE=1`. Hit each endpoint
  once, assert response shape. Never run in CI by default to respect
  the upstream service.
- **Manifest round-trip:** given a known tone id, the generated
  manifest must serialise+deserialise and match the structure that
  `pack_plugins` expects (re-parse via the same yaml deserialiser).

## Open questions (acknowledge, defer to plan)

- **Anon-JWT rotation cadence.** Hardcoded value is valid until 2035-02-06,
  but Supabase may rotate sooner. The refresh-from-homepage fallback
  covers this; observability could be a one-line log if the fallback
  fires.
- **Parameter-classifier corpus.** The first version learns from current
  manifests; new capture-name conventions on tone3000 will need
  dictionary updates. Acceptable to ship narrow and grow.
- **License surfacing.** Each tone has a license (`t3k`, `cc-by`, etc.)
  that needs to land somewhere — manifest field? Out of scope here;
  open a follow-up issue.

## Non-goals (deliberate restrictions)

- The binary writes **drafts**. It never edits an existing
  `manifest.yaml`. Re-importing the same tone id over an existing
  directory aborts with a clear error pointing at the conflict.
- The skill never bypasses the gate. `QA_AUDIT_SKIP=1` is not a path
  this skill ever sets; if `qa_audit` fails on the imported pack, the
  user fixes the data (or relaxes a calibrated threshold in `qa.rs`),
  not the audit toggle.

## Out of scope

- Authenticated tone3000 actions (favoriting, uploading own tones).
- Bulk import (more than one tone per command). One issue = one tone
  pack — keeps PRs reviewable.
- Browser MCP integration. The Rust binary is self-contained and
  callable from CI; a browser is not on the dependency surface.

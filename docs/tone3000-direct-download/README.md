# Idea — stop bundling tone3000 captures, download them from tone3000

Status: idea (owner, 2026-09-30). Not scheduled.

## Why

While filling plugin metadata (#153) we found that 316 of the shipped tones use
tone3000's `T3K` license. Its terms, as shown on each tone page: users may
download and use the file and publish the outputs, but may not upload,
republish or distribute the data file without the author's permission.
This repo redistributes those `.nam`/`.wav` files inside every OpenRig release.

## Direction

- Remove the tone3000-sourced captures (`.nam` / IR `.wav`) from
  `plugins/source/nam` and `plugins/source/ir`.
- The user downloads each capture straight from tone3000, so the file comes
  from the author's own publication, never from us.
- What we can still ship is our own work: the manifest (display name, brand,
  real-control parameters, `sources`, `output_gain_db`, noise gate, metadata).

## Open questions

- How OpenRig fetches the files: tone3000 API / OAuth flow
  (<https://github.com/tone-3000/api>; free for non-commercial, open-source
  products under their API terms) vs. the user downloading by hand.
- Whether `output_gain_db` / noise-gate values measured on a file still match
  if the author re-trains the tone (`scripts/check_updates.py` already
  fingerprints tone content).
- What happens to the 12 tones already no longer public on tone3000, and to
  the `CC BY` / `CC0` tones, which may stay bundled.
- Legacy captures with no recorded source (169 manifests): origin unknown, so
  their redistribution status is unknown too.
- OpenRig side: the app needs the download/install flow; that work belongs to
  the OpenRig repo.

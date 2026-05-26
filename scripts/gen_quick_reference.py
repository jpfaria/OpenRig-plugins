#!/usr/bin/env python3
"""Regenerate the Quick Reference block in `docs/blocks-reference.md`.

Reads every `plugins/source/{nam,ir,lv2}/*/manifest.yaml` for canonical
`MODEL_ID`s and pairs them with the engine-side native models listed
in `scripts/native_models.yaml`. Emits the full Quick Reference section
between the markers:

    <!-- QUICK-REFERENCE-START -->
    ...auto-generated...
    <!-- QUICK-REFERENCE-END -->

The deeper per-section catalogues (descriptions, parameter tables) and
the doc prose around the markers are untouched. Re-run after adding,
removing, or renaming a plugin manifest.

Usage:
    scripts/gen_quick_reference.py            # rewrite the doc in place
    scripts/gen_quick_reference.py --check    # exit 1 if it would change
"""
from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DOC = REPO / "docs" / "blocks-reference.md"
PLUGINS = REPO / "plugins" / "source"
NATIVE = Path(__file__).resolve().parent / "native_models.yaml"

# `type:` value on a manifest → Quick Reference section it belongs to.
# Anything outside this map is documented in deeper sections of the
# doc and skipped here.
TYPE_TO_SECTION = {
    "preamp": "preamp",
    "amp": "amp",
    "cab": "cab",
    "body": "body",
    "gain_pedal": "gain",
}

# Display order + section heading + introductory blurb.
SECTION_HEADINGS = [
    ("preamp", "Preamp", "Preamp blocks model the early gain stage of an amp head."),
    ("amp", "Amp", "Full amp blocks include preamp + power amp; pair with a cab IR for the full chain."),
    ("cab", "Cab", "Cabinet impulse responses for electric guitar speakers."),
    ("body", "Body", "Acoustic body impulse responses for piezo / magnetic pickup emulation."),
    ("gain", "Gain", "Boost / overdrive / distortion / fuzz / volume pedals."),
]

MARKER_START = "<!-- QUICK-REFERENCE-START -->"
MARKER_END = "<!-- QUICK-REFERENCE-END -->"


def yaml_scalar(line: str, key: str) -> str | None:
    if line.startswith(f"{key}:"):
        return line.split(":", 1)[1].strip().strip("'\"")
    return None


def parse_manifest_head(path: Path) -> dict[str, str]:
    """Pulls the top-level scalar fields (`id`, `type`, `display_name`,
    `brand`) without a YAML library — manifests are flat enough that
    this is reliable and dependency-free."""
    fields: dict[str, str] = {}
    for line in path.read_text().splitlines():
        if line.startswith(" ") or line.startswith("\t"):
            continue
        for key in ("id", "type", "display_name", "brand"):
            if key in fields:
                continue
            v = yaml_scalar(line, key)
            if v is not None:
                fields[key] = v
    return fields


def load_native() -> dict[str, list[tuple[str, str, str]]]:
    """Hand-curated native models live in YAML; parse without PyYAML so
    the script has no third-party deps."""
    out: dict[str, list[tuple[str, str, str]]] = defaultdict(list)
    section: str | None = None
    item: dict[str, str] = {}
    for raw in NATIVE.read_text().splitlines():
        s = raw.rstrip()
        if not s or s.startswith("#"):
            continue
        if not s.startswith(" "):
            # Top-level key.
            if item and section:
                out[section].append(
                    (item.get("id", ""), item.get("display_name", "--"), item.get("brand", "--"))
                )
                item = {}
            m = re.match(r"(\w+):\s*(\[\])?$", s)
            if m:
                section = m.group(1)
                continue
            section = None
            continue
        if s.lstrip().startswith("- "):
            if item and section:
                out[section].append(
                    (item.get("id", ""), item.get("display_name", "--"), item.get("brand", "--"))
                )
                item = {}
            kv = s.lstrip()[2:]
            k, _, v = kv.partition(":")
            item[k.strip()] = v.strip().strip("'\"")
        else:
            k, _, v = s.lstrip().partition(":")
            item[k.strip()] = v.strip().strip("'\"")
    if item and section:
        out[section].append(
            (item.get("id", ""), item.get("display_name", "--"), item.get("brand", "--"))
        )
    return out


def collect_plugins() -> dict[str, list[tuple[str, str, str]]]:
    rows: dict[str, list[tuple[str, str, str]]] = defaultdict(list)
    for kind_dir in sorted(PLUGINS.iterdir()):
        if not kind_dir.is_dir():
            continue
        for plugin_dir in sorted(kind_dir.iterdir()):
            manifest = plugin_dir / "manifest.yaml"
            if not manifest.is_file():
                continue
            f = parse_manifest_head(manifest)
            section = TYPE_TO_SECTION.get(f.get("type", ""))
            if section is None or "id" not in f:
                continue
            rows[section].append(
                (f["id"], f.get("display_name", "--"), f.get("brand", "--"))
            )
    return rows


def render() -> str:
    native = load_native()
    plugin = collect_plugins()
    out: list[str] = []
    for key, heading, blurb in SECTION_HEADINGS:
        merged = native.get(key, []) + plugin.get(key, [])
        merged.sort(key=lambda r: r[0])
        out.append(f"### {heading}")
        out.append("")
        out.append(f"{blurb}")
        out.append("")
        out.append("| Model ID | Display Name | Brand |")
        out.append("|---|---|---|")
        for mid, display, brand in merged:
            out.append(f"| `{mid}` | {display} | {brand} |")
        out.append("")
    return "\n".join(out)


def patch(doc_text: str, generated: str) -> str:
    pattern = re.compile(
        rf"({re.escape(MARKER_START)}\n)(.*?)(\n{re.escape(MARKER_END)})",
        re.DOTALL,
    )
    return pattern.sub(lambda m: f"{m.group(1)}{generated}{m.group(3)}", doc_text)


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--check", action="store_true", help="exit 1 if the doc would change")
    args = p.parse_args()

    if not DOC.is_file():
        print(f"error: {DOC} not found", file=sys.stderr)
        return 2
    current = DOC.read_text()
    if MARKER_START not in current or MARKER_END not in current:
        print(
            f"error: doc missing {MARKER_START} / {MARKER_END} markers; "
            "add them around the Quick Reference block before running.",
            file=sys.stderr,
        )
        return 2

    updated = patch(current, render())
    if args.check:
        if current != updated:
            print(
                "docs/blocks-reference.md is out of date — run scripts/gen_quick_reference.py",
                file=sys.stderr,
            )
            return 1
        return 0
    if current != updated:
        DOC.write_text(updated)
        print(f"updated {DOC.relative_to(REPO)}")
    else:
        print("no changes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

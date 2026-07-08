#!/usr/bin/env python3
"""check_updates.py — report outdated plugins.

Two independent checkers behind one CLI:

  --submodules  LV2/VST3 built from deps/ submodules: the pinned tag/branch vs
                the upstream (git ls-remote). Deterministic.
  --tone3000    NAM/IR captures: the source tone gained models or was re-trained
                (content-hash fingerprint vs scripts/.update_state.json).

No flag -> run both. Report-only: never mutates manifests, never touches shared
state (no issues/PRs). Exit 0 by default; --fail-on-outdated opts into non-zero.

Network (git ls-remote, the tone3000 API) needs the sandbox off.
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys
import urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

_TAG_RE = re.compile(r"^(v?)(\d+(?:\.\d+)*)$")


# --------------------------------------------------------------------------- #
# Checker A — submodules (LV2 + VST3): pure parsers                           #
# --------------------------------------------------------------------------- #
def parse_gitmodules(text):
    """[(path, url)] from a .gitmodules body."""
    path = url = None
    out = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("path ="):
            path = line.split("=", 1)[1].strip()
        elif line.startswith("url ="):
            url = line.split("=", 1)[1].strip()
        if path and url:
            out.append((path, url))
            path = url = None
    return out


def classify_pin(status_line):
    """{sha, descriptor} from a `git submodule status` line.

    Only the pinned SHA is authoritative — the parenthesised describe is absent
    for uninitialised submodules (e.g. a fresh clone), so "is this pin on a
    tag?" is decided against the REMOTE tag list, not this descriptor.
    """
    s = status_line.strip()
    sha = re.sub(r"^[-+ ]", "", s).split()[0]
    m = re.search(r"\(([^)]+)\)\s*$", s)
    return {"sha": sha, "descriptor": m.group(1) if m else None}


def version_key(tag):
    """Sortable tuple from a tag like v2.11.4 / 0.9.8, else None."""
    m = _TAG_RE.match(tag.strip())
    if not m:
        return None
    return tuple(int(x) for x in m.group(2).split("."))


def pick_newer_tag(current, remote_tags):
    """Newest tag strictly greater than `current` sharing its numeric shape."""
    cur = version_key(current)
    if cur is None:
        return None
    best = None
    best_key = cur
    for t in remote_tags:
        k = version_key(t)
        if k is not None and len(k) == len(cur) and k > best_key:
            best, best_key = t, k
    return best


# --------------------------------------------------------------------------- #
# Checker A — submodule -> plugins mapping + decision + IO                     #
# --------------------------------------------------------------------------- #
# recipe name (scripts/plugin-recipes.tsv column 2) -> deps/ submodule path(s).
# Mirrors the recipe functions in scripts/build-lib-internal.sh.
RECIPE_SUBMODULE = {
    "aether": ["deps/Aether"],
    "artyfx": ["deps/openAV-ArtyFX"],
    "bolliedelay": ["deps/DISTRHO-Ports"],
    "caps-lv2": ["deps/caps-lv2"],
    "chowcentaur": ["deps/KlonCentaur"],
    "distrho": ["deps/DPF-Plugins", "deps/MVerb"],
    "dragonfly-reverb": ["deps/dragonfly-reverb"],
    "fomp": ["deps/fomp"],
    "gxplugins": ["deps/GxPlugins.lv2"],
    "invada-studio": ["deps/invada-studio"],
    "mda-lv2": ["deps/mda-lv2"],
    "mod-utilities": ["deps/mod-utilities"],
    "mverb": ["deps/MVerb"],
    "ojd": ["deps/Schrammel_OJD"],
    "setbfree": ["deps/setBfree"],
    "shiro-plugins": ["deps/SHIRO-Plugins"],
    "tap-lv2": ["deps/tap-lv2"],
    "wolf-shaper": ["deps/wolf-shaper"],
    "x42": ["deps/darc.lv2", "deps/dpl.lv2", "deps/fil4.lv2"],
    "zam-plugins": ["deps/zam-plugins"],
}


def load_recipe_plugins(tsv_text):
    """recipe -> [plugin folder] from plugin-recipes.tsv (skips comments, '-')."""
    out = {}
    for line in tsv_text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 2:
            continue
        folder, recipe = parts[0].strip(), parts[1].strip()
        if recipe == "-" or not recipe:
            continue
        out.setdefault(recipe, []).append(folder)
    return out


def submodule_to_plugins(path, recipe_plugins, recipe_submodule):
    plugins = []
    for recipe, paths in recipe_submodule.items():
        if path in paths:
            plugins.extend(recipe_plugins.get(recipe, []))
    return sorted(set(plugins))


def find_current_tag(pin_sha, tag_shas):
    """The tag whose commit is exactly the pinned SHA, else None."""
    for tag, sha in tag_shas.items():
        if sha == pin_sha:
            return tag
    return None


def decide_submodule(pin_sha, remote_head, tag_shas):
    """(state, detail); state in {current, behind, new-tag}.

    Pin sits exactly on a release tag -> flag only when a newer tag exists.
    Pin sits on a branch / non-tag commit -> flag when behind the branch HEAD.
    `tag_shas` maps tag name -> the commit SHA it points to (peeled).
    """
    cur_tag = find_current_tag(pin_sha, tag_shas)
    if cur_tag:
        newer = pick_newer_tag(cur_tag, list(tag_shas.keys()))
        if newer:
            return "new-tag", f"{cur_tag} -> {newer}"
        return "current", cur_tag
    if remote_head and remote_head != pin_sha:
        return "behind", f"{pin_sha[:8]} -> {remote_head[:8]}"
    return "current", pin_sha[:8]


def _git(args):
    return subprocess.run(["git", "-C", REPO] + args,
                          capture_output=True, text=True).stdout


def _ls_remote(url):
    """(default-branch HEAD sha, {tag name: commit sha}) for a remote url.

    Annotated tags expose the tag object under `refs/tags/<t>` and the commit
    it points to under `refs/tags/<t>^{}`; the peeled `^{}` line, when present,
    is the authoritative commit for matching against a submodule pin.
    """
    tags_out = subprocess.run(["git", "ls-remote", "--tags", url],
                              capture_output=True, text=True, timeout=60).stdout
    tag_shas = {}
    for line in tags_out.splitlines():
        sha, _, ref = line.partition("\t")
        if not ref.startswith("refs/tags/"):
            continue
        name = ref[len("refs/tags/"):]
        if name.endswith("^{}"):
            tag_shas[name[:-3]] = sha.strip()          # peeled commit wins
        else:
            tag_shas.setdefault(name, sha.strip())     # lightweight tag
    head_out = subprocess.run(["git", "ls-remote", url, "HEAD"],
                              capture_output=True, text=True, timeout=60).stdout
    head = head_out.split()[0] if head_out.strip() else ""
    return head, tag_shas


def check_submodules(ls_remote=_ls_remote):
    mods = parse_gitmodules(open(os.path.join(REPO, ".gitmodules")).read())
    status = _git(["submodule", "status"])
    pins = {}
    for line in status.splitlines():
        if not line.strip():
            continue
        m = re.search(r"deps/[^\s(]+", line)
        if m:
            pins[m.group(0)] = classify_pin(line)
    rp = load_recipe_plugins(
        open(os.path.join(REPO, "scripts/plugin-recipes.tsv")).read())
    rows = []
    for path, url in mods:
        pin = pins.get(path)
        if not pin:
            continue
        plugins = submodule_to_plugins(path, rp, RECIPE_SUBMODULE)
        try:
            head, tag_shas = ls_remote(url)
        except Exception as e:  # noqa: BLE001 - report, never crash the run
            rows.append({"path": path, "state": "err", "detail": str(e),
                         "plugins": plugins})
            continue
        state, detail = decide_submodule(pin["sha"], head, tag_shas)
        rows.append({"path": path, "state": state, "detail": detail,
                     "plugins": plugins})
    return rows


# --------------------------------------------------------------------------- #
# Checker B — tone3000 (NAM + IR): pure functions                             #
# --------------------------------------------------------------------------- #
def parse_tone_ids(manifest_text):
    return re.findall(r"tone3000\.com/tones/(\d+)", manifest_text)


def count_captures(manifest_text):
    """Count `file:` entries under the top-level `captures:` block."""
    in_caps = False
    n = 0
    for line in manifest_text.splitlines():
        if re.match(r"^captures:\s*$", line):
            in_caps = True
            continue
        if in_caps:
            # end of the block: a new top-level key (non-space, not a `- ` item)
            if line and not line[0].isspace() and not line.startswith("-"):
                break
            if re.search(r"\bfile:\s*\S", line):
                n += 1
    return n


def fingerprint(models):
    """Sorted set of model_url basenames (each basename is a content hash)."""
    return sorted(os.path.basename(m["model_url"])
                  for m in models if m.get("model_url"))


def distinct_captures(fp):
    """Capture identities in a fingerprint, collapsing the A1/A2 pair.

    tone3000 exposes both `<hash>.nam` (A1) and `<hash>_a2.nam` (A2) for the
    same capture, so a raw count double-counts. Strip the extension and the
    `_a2` suffix to recover the real number of distinct captures.
    """
    bases = set()
    for name in fp:
        b = name.rsplit(".", 1)[0]
        if b.endswith("_a2"):
            b = b[:-3]
        bases.add(b)
    return bases


def tone_flags(current_fp, prev_fp, imported_count):
    flags = []
    cur, prev = set(current_fp), set(prev_fp or [])
    if len(distinct_captures(current_fp)) > imported_count:
        flags.append("new-models")
    if prev_fp is not None and cur != prev:
        flags.append("changed-since-last-check")
    if prev_fp is not None and (prev - cur):
        flags.append("removed-upstream")
    return flags


# --------------------------------------------------------------------------- #
# Checker B — tone3000 HTTP + state cache                                      #
# --------------------------------------------------------------------------- #
# public anon read key — same value as scripts/param_gate.py (source of truth)
TONE3000_TOKEN = (
    "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9."
    "eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Imd6eWJpdW9weGtkeGJ5dG5vamRzIiwicm9sZSI6"
    "ImFub24iLCJpYXQiOjE3MzgwODIxNjUsImV4cCI6MjA1MzY1ODE2NX0."
    "Gq66BJXjtLsqP2nAGXm9Xb9PAjoeZalWUj66K4nmVSU"
)
STATE_PATH = os.path.join(REPO, "scripts", ".update_state.json")


def fetch_models(tone_id):
    url = (f"https://api.tone3000.com/rest/v1/models?tone_id=eq.{tone_id}"
           f"&select=name,model_url")
    req = urllib.request.Request(url, headers={
        "apikey": TONE3000_TOKEN, "Authorization": "Bearer " + TONE3000_TOKEN})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


def load_state(path):
    try:
        return json.load(open(path))
    except (FileNotFoundError, ValueError):
        return {}


def save_state(path, state):
    with open(path, "w") as f:
        json.dump(state, f, indent=0, sort_keys=True)
        f.write("\n")


def _iter_manifests():
    for kind in ("nam", "ir"):
        pattern = os.path.join(REPO, "plugins/source", kind, "*", "manifest.yaml")
        for mf in sorted(glob.glob(pattern)):
            folder = f"{kind}/{os.path.basename(os.path.dirname(mf))}"
            yield folder, open(mf).read()


def check_tone3000(fetch=fetch_models, manifests=None, state=None):
    """(rows, new_state). rows include `unchecked` for sourceless manifests."""
    if manifests is None:
        manifests = list(_iter_manifests())
    if state is None:
        state = load_state(STATE_PATH)
    new_state = dict(state)
    rows = []
    for folder, text in manifests:
        ids = parse_tone_ids(text)
        if not ids:
            rows.append({"folder": folder, "state": "unchecked",
                         "detail": "no provenance (no sources:)", "flags": []})
            continue
        imported = count_captures(text)
        for tid in dict.fromkeys(ids):  # unique, order-preserving
            try:
                models = fetch(tid)
            except Exception as e:  # noqa: BLE001 - report, never crash the run
                rows.append({"folder": folder, "tone_id": tid, "state": "err",
                             "detail": str(e), "flags": []})
                continue
            fp = fingerprint(models)
            prev = state.get(tid, {}).get("fingerprint")
            flags = tone_flags(fp, prev, imported)
            rows.append({"folder": folder, "tone_id": tid,
                         "state": "outdated" if flags else "current",
                         "flags": flags,
                         "detail": f"{len(fp)} models, {imported} imported"})
            new_state[tid] = {"fingerprint": fp}
    return rows, new_state


# --------------------------------------------------------------------------- #
# CLI                                                                          #
# --------------------------------------------------------------------------- #
def render_table(sub_rows, tone_rows):
    lines = []
    if sub_rows is not None:
        lines.append("== submodules (LV2/VST3) ==")
        for r in sorted(sub_rows,
                        key=lambda x: (x["state"] != "current", x["path"])):
            plugins = ",".join(r.get("plugins", [])) or "-"
            lines.append(f"{r['state']:<9} {r['path']:<26} "
                         f"{r['detail']:<24} {plugins}")
    if tone_rows is not None:
        if lines:
            lines.append("")
        lines.append("== tone3000 (NAM/IR) ==")
        for r in sorted(tone_rows,
                        key=lambda x: (x["state"] == "current",
                                       x["state"] == "unchecked",
                                       x.get("folder", ""))):
            tag = ",".join(r.get("flags", [])) or r.get("detail", "")
            lines.append(f"{r['state']:<10} {r.get('folder', ''):<40} "
                         f"{r.get('tone_id', ''):<8} {tag}")
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Report outdated plugins.")
    ap.add_argument("--submodules", action="store_true",
                    help="check LV2/VST3 submodules only")
    ap.add_argument("--tone3000", action="store_true",
                    help="check NAM/IR tone3000 sources only")
    ap.add_argument("--json", action="store_true", help="emit JSON")
    ap.add_argument("--fail-on-outdated", action="store_true",
                    help="exit 1 if anything is flagged")
    ap.add_argument("--no-write-state", action="store_true",
                    help="do not update scripts/.update_state.json")
    args = ap.parse_args(argv)
    do_sub = args.submodules or not args.tone3000
    do_tone = args.tone3000 or not args.submodules

    sub_rows = check_submodules() if do_sub else None
    tone_rows = None
    if do_tone:
        tone_rows, new_state = check_tone3000()
        if not args.no_write_state:
            save_state(STATE_PATH, new_state)

    if args.json:
        print(json.dumps({"submodules": sub_rows, "tone3000": tone_rows},
                         indent=2))
    else:
        print(render_table(sub_rows, tone_rows))

    outdated = any(r["state"] != "current" for r in (sub_rows or [])) or \
        any(r["state"] not in ("current", "unchecked")
            for r in (tone_rows or []))
    return 1 if (args.fail_on_outdated and outdated) else 0


if __name__ == "__main__":
    sys.exit(main())

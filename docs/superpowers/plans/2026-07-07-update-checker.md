# Update-Checker Mechanism Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `scripts/check_updates.py` reporter that flags outdated LV2/VST3 submodules and outdated tone3000 NAM/IR captures, plus a skill that drives it.

**Architecture:** One stdlib-only Python CLI with two independent checkers of pure functions (parse/classify/compare) wrapped by thin git/HTTP IO layers. Submodule drift is deterministic (`git ls-remote` vs pinned tag/branch); tone3000 drift is best-effort via a content-hash fingerprint cached in `scripts/.update_state.json`. Report-only, exit 0.

**Tech Stack:** Python 3.12 stdlib (`urllib`, `json`, `subprocess`, `re`, `argparse`, `unittest`). No third-party deps (no PyYAML). tests via `unittest`.

## Global Constraints

- Language: English everywhere (code, comments, docs, commits). No `Co-Authored-By`, no `Fixes #`.
- Stdlib only — no pip installs. Manifests parsed by regex, not PyYAML.
- tone3000 API: `https://api.tone3000.com/rest/v1/models?tone_id=eq.<id>&select=name,model_url`, headers `apikey` + `Authorization: Bearer <token>`; the token is the public anon key already embedded in `scripts/param_gate.py` — replicate it with a comment naming that file as the source of truth.
- Report-only: never mutate manifests; never open issues/PRs. Exit 0 by default; `--fail-on-outdated` opts into non-zero.
- Slot invariant untouched (no manifest/slot surface). Gate `cargo run --release --bin pack_plugins` → exit 0 before push.
- All work in `.solvers/issue-107/`, branch `feature/issue-107`. Every push commented on issue #107.
- Tests run: `python3 scripts/test_check_updates.py` (unittest `main()`).

---

### Task 1: Pure submodule parsers + tag comparison

**Files:**
- Create: `scripts/check_updates.py`
- Test: `scripts/test_check_updates.py`

**Interfaces:**
- Produces:
  - `parse_gitmodules(text: str) -> list[tuple[str, str]]` — `[(path, url)]`.
  - `classify_pin(status_line: str) -> dict` — `{"sha": str, "descriptor": str|None, "on_tag": bool, "tag": str|None}` from a `git submodule status` line.
  - `version_key(tag: str) -> tuple` — sortable key from a tag like `v2.11.4` / `0.9.8`.
  - `pick_newer_tag(current: str, remote_tags: list[str]) -> str|None` — newest tag strictly greater than `current` sharing its numeric shape, else `None`.

- [ ] **Step 1: Write the failing tests**

```python
import unittest
import check_updates as cu

class TestSubmoduleParsing(unittest.TestCase):
    def test_parse_gitmodules(self):
        text = (
            '[submodule "deps/Aether"]\n'
            '\tpath = deps/Aether\n'
            '\turl = https://github.com/Dougal-s/Aether.git\n'
        )
        self.assertEqual(cu.parse_gitmodules(text),
                         [("deps/Aether", "https://github.com/Dougal-s/Aether.git")])

    def test_classify_pin_on_tag(self):
        line = " 604372e4ffd9690c3e283362e4598cb43edbb475 deps/AnalogTapeModel (v2.11.4)"
        p = cu.classify_pin(line)
        self.assertEqual(p["sha"], "604372e4ffd9690c3e283362e4598cb43edbb475")
        self.assertTrue(p["on_tag"])
        self.assertEqual(p["tag"], "v2.11.4")

    def test_classify_pin_on_branch(self):
        line = " 3e0a1d35f6ddfe3430e5921e0d55cf60574f8bc3 deps/SHIRO-Plugins (heads/master)"
        self.assertFalse(cu.classify_pin(line)["on_tag"])

    def test_classify_pin_commits_past_tag_is_not_on_tag(self):
        line = " df5cb658a2d8dc6e944d8c26a53853a7c0c6c2b2 deps/DPF-Plugins (v1.7-15-gdf5cb65)"
        self.assertFalse(cu.classify_pin(line)["on_tag"])

    def test_pick_newer_tag(self):
        self.assertEqual(cu.pick_newer_tag("v2.11.4", ["v2.11.3", "v2.11.4", "v2.12.0"]), "v2.12.0")
        self.assertIsNone(cu.pick_newer_tag("v2.11.4", ["v2.11.3", "v2.11.4"]))

if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd scripts && python3 test_check_updates.py -v`
Expected: FAIL — `AttributeError: module 'check_updates' has no attribute ...`

- [ ] **Step 3: Implement the pure functions**

In `scripts/check_updates.py`:

```python
#!/usr/bin/env python3
"""check_updates.py — report outdated plugins.

Two checkers:
  --submodules  LV2/VST3 built from deps/ submodules: pinned tag/branch vs upstream.
  --tone3000    NAM/IR captures: source tone re-trained / gained models.
No flag -> both. Report-only, exit 0 (unless --fail-on-outdated).
Network (git ls-remote, tone3000 API) needs the sandbox off.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import urllib.request

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

_TAG_RE = re.compile(r"^(v?)(\d+(?:\.\d+)*)$")

def parse_gitmodules(text):
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
    # e.g. " <sha> deps/AnalogTapeModel (v2.11.4)"  or "-<sha> deps/Aether"
    s = status_line.strip()
    sha = re.sub(r"^[-+ ]", "", s).split()[0]
    m = re.search(r"\(([^)]+)\)\s*$", s)
    descriptor = m.group(1) if m else None
    # on a tag exactly when the descriptor is a bare tag (no '-N-g<hash>' suffix,
    # not a 'heads/...' branch ref).
    on_tag = bool(descriptor) and not descriptor.startswith("heads/") \
        and not re.search(r"-\d+-g[0-9a-f]+$", descriptor)
    return {"sha": sha, "descriptor": descriptor,
            "on_tag": on_tag, "tag": descriptor if on_tag else None}

def version_key(tag):
    m = _TAG_RE.match(tag.strip())
    if not m:
        return None
    return tuple(int(x) for x in m.group(2).split("."))

def pick_newer_tag(current, remote_tags):
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd scripts && python3 test_check_updates.py -v`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
cd .solvers/issue-107
git add scripts/check_updates.py scripts/test_check_updates.py
git commit -m "feat(check_updates): submodule pin parsers + tag comparison (#107)"
```

---

### Task 2: Submodule checker (git ls-remote) + recipe→plugins mapping

**Files:**
- Modify: `scripts/check_updates.py`
- Test: `scripts/test_check_updates.py`

**Interfaces:**
- Consumes: `parse_gitmodules`, `classify_pin`, `pick_newer_tag`.
- Produces:
  - `RECIPE_SUBMODULE: dict[str, list[str]]` — recipe name → submodule path(s).
  - `load_recipe_plugins(tsv_text: str) -> dict[str, list[str]]` — recipe → plugin folders.
  - `submodule_to_plugins(path: str, recipe_plugins, recipe_submodule) -> list[str]`.
  - `decide_submodule(pin: dict, remote_head: str, remote_tags: list[str]) -> tuple[str, str]` — `(state, detail)`, `state ∈ {"current","behind","new-tag"}`.
  - `check_submodules(run=..., ls_remote=...) -> list[dict]` — IO wrapper (injectable for tests).

- [ ] **Step 1: Write the failing tests**

```python
class TestSubmoduleDecision(unittest.TestCase):
    def test_decide_on_tag_newer_available(self):
        pin = {"on_tag": True, "tag": "v2.11.4", "sha": "aaa"}
        state, detail = cu.decide_submodule(pin, "deadbeef", ["v2.11.4", "v2.12.0"])
        self.assertEqual(state, "new-tag"); self.assertIn("v2.12.0", detail)

    def test_decide_on_tag_up_to_date(self):
        pin = {"on_tag": True, "tag": "v2.11.4", "sha": "aaa"}
        self.assertEqual(cu.decide_submodule(pin, "x", ["v2.11.4"])[0], "current")

    def test_decide_branch_behind(self):
        pin = {"on_tag": False, "tag": None, "sha": "aaa"}
        self.assertEqual(cu.decide_submodule(pin, "bbb", [])[0], "behind")

    def test_decide_branch_current(self):
        pin = {"on_tag": False, "tag": None, "sha": "aaa"}
        self.assertEqual(cu.decide_submodule(pin, "aaa", [])[0], "current")

    def test_load_recipe_plugins(self):
        tsv = "# comment\naether\taether\nb_reverb\tsetbfree\navocado\t-\n"
        m = cu.load_recipe_plugins(tsv)
        self.assertEqual(m["aether"], ["aether"])
        self.assertEqual(m["setbfree"], ["b_reverb"])
        self.assertNotIn("-", m)

    def test_submodule_to_plugins(self):
        rp = {"aether": ["aether"]}
        rs = {"aether": ["deps/Aether"]}
        self.assertEqual(cu.submodule_to_plugins("deps/Aether", rp, rs), ["aether"])
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd scripts && python3 test_check_updates.py -v`
Expected: FAIL — attributes missing.

- [ ] **Step 3: Implement decision + mapping + IO wrapper**

```python
# recipe name (scripts/plugin-recipes.tsv col 2) -> deps/ submodule path(s).
# Source of truth mirrors scripts/build-lib-internal.sh recipe functions.
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

def decide_submodule(pin, remote_head, remote_tags):
    if pin["on_tag"]:
        newer = pick_newer_tag(pin["tag"], remote_tags)
        if newer:
            return "new-tag", f"{pin['tag']} -> {newer}"
        return "current", pin["tag"]
    if remote_head and remote_head != pin["sha"]:
        return "behind", f"{pin['sha'][:8]} -> {remote_head[:8]}"
    return "current", pin["sha"][:8]

def _git(args):
    return subprocess.run(["git", "-C", REPO] + args,
                          capture_output=True, text=True).stdout

def _ls_remote(url):
    out = subprocess.run(["git", "ls-remote", "--heads", "--tags", url],
                         capture_output=True, text=True, timeout=60).stdout
    head = ""
    tags = []
    for line in out.splitlines():
        sha, _, ref = line.partition("\t")
        if ref.startswith("refs/tags/") and not ref.endswith("^{}"):
            tags.append(ref[len("refs/tags/"):])
    head_out = subprocess.run(["git", "ls-remote", url, "HEAD"],
                              capture_output=True, text=True, timeout=60).stdout
    if head_out:
        head = head_out.split()[0]
    return head, tags

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
    rp = load_recipe_plugins(open(os.path.join(REPO, "scripts/plugin-recipes.tsv")).read())
    rows = []
    for path, url in mods:
        pin = pins.get(path)
        if not pin:
            continue
        try:
            head, tags = ls_remote(url)
        except Exception as e:  # noqa: BLE001 - report, never crash the run
            rows.append({"path": path, "state": "err", "detail": str(e),
                         "plugins": submodule_to_plugins(path, rp, RECIPE_SUBMODULE)})
            continue
        state, detail = decide_submodule(pin, head, tags)
        rows.append({"path": path, "state": state, "detail": detail,
                     "plugins": submodule_to_plugins(path, rp, RECIPE_SUBMODULE)})
    return rows
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd scripts && python3 test_check_updates.py -v`
Expected: PASS (all).

- [ ] **Step 5: Commit**

```bash
git add scripts/check_updates.py scripts/test_check_updates.py
git commit -m "feat(check_updates): submodule checker + recipe->plugins mapping (#107)"
```

---

### Task 3: tone3000 fingerprint pure functions

**Files:**
- Modify: `scripts/check_updates.py`
- Test: `scripts/test_check_updates.py`

**Interfaces:**
- Produces:
  - `parse_tone_ids(manifest_text: str) -> list[str]` — tone ids from `sources:`.
  - `count_captures(manifest_text: str) -> int` — `file:` lines under `captures:`.
  - `fingerprint(models: list[dict]) -> list[str]` — sorted `model_url` basenames.
  - `tone_flags(current_fp, prev_fp, imported_count) -> list[str]` — subset of `{"new-models","changed-since-last-check","removed-upstream"}`.

- [ ] **Step 1: Write the failing tests**

```python
class TestToneFingerprint(unittest.TestCase):
    def test_parse_tone_ids(self):
        text = "sources:\n- https://www.tone3000.com/tones/5196\n"
        self.assertEqual(cu.parse_tone_ids(text), ["5196"])

    def test_count_captures(self):
        text = "captures:\n- file: a.nam\n- file: b.nam\n"
        self.assertEqual(cu.count_captures(text), 2)

    def test_fingerprint(self):
        models = [{"model_url": "x/aa.nam"}, {"model_url": "y/bb.nam"}]
        self.assertEqual(cu.fingerprint(models), ["aa.nam", "bb.nam"])

    def test_flags_new_models(self):
        self.assertIn("new-models", cu.tone_flags(["a", "b", "c"], ["a", "b", "c"], 2))

    def test_flags_changed(self):
        self.assertIn("changed-since-last-check", cu.tone_flags(["a", "z"], ["a", "b"], 2))

    def test_flags_removed(self):
        self.assertIn("removed-upstream", cu.tone_flags(["a"], ["a", "b"], 2))

    def test_flags_none_when_stable(self):
        self.assertEqual(cu.tone_flags(["a", "b"], ["a", "b"], 2), [])
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd scripts && python3 test_check_updates.py -v` → FAIL.

- [ ] **Step 3: Implement**

```python
def parse_tone_ids(manifest_text):
    return re.findall(r"tone3000\.com/tones/(\d+)", manifest_text)

def count_captures(manifest_text):
    in_caps = False
    n = 0
    for line in manifest_text.splitlines():
        if re.match(r"^captures:\s*$", line):
            in_caps = True
            continue
        if in_caps:
            if re.match(r"^\S", line):  # dedented to a new top-level key
                break
            if re.search(r"\bfile:\s*\S", line):
                n += 1
    return n

def fingerprint(models):
    return sorted(os.path.basename(m["model_url"]) for m in models if m.get("model_url"))

def tone_flags(current_fp, prev_fp, imported_count):
    flags = []
    cur, prev = set(current_fp), set(prev_fp or [])
    if len(current_fp) > imported_count:
        flags.append("new-models")
    if prev_fp is not None and cur != prev:
        flags.append("changed-since-last-check")
    if prev_fp is not None and (prev - cur):
        flags.append("removed-upstream")
    return flags
```

- [ ] **Step 4: Run tests** → PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/check_updates.py scripts/test_check_updates.py
git commit -m "feat(check_updates): tone3000 fingerprint + flags (#107)"
```

---

### Task 4: tone3000 checker (HTTP) + state cache

**Files:**
- Modify: `scripts/check_updates.py`
- Test: `scripts/test_check_updates.py`

**Interfaces:**
- Consumes: `parse_tone_ids`, `count_captures`, `fingerprint`, `tone_flags`.
- Produces:
  - `TONE3000_TOKEN` — public anon key (comment: same as `scripts/param_gate.py`).
  - `fetch_models(tone_id: str) -> list[dict]` — HTTP GET.
  - `load_state(path) -> dict`, `save_state(path, state) -> None`.
  - `check_tone3000(fetch=fetch_models, state=None) -> tuple[list[dict], dict]` — `(rows, new_state)`; rows include `unchecked` for manifests without `sources:`.

- [ ] **Step 1: Write the failing tests** (inject `fetch`, no network)

```python
class TestToneChecker(unittest.TestCase):
    def test_check_tone3000_flags_and_state(self):
        # fake manifest dir via monkeypatch of the glob helper
        fake = {"5196": [{"model_url": "x/aa.nam"}, {"model_url": "y/bb.nam"},
                          {"model_url": "z/cc.nam"}]}
        manifests = [("nam/klon", "sources:\n- t.com/tones/5196\ncaptures:\n- file: a.nam\n- file: b.nam\n")]
        rows, new_state = cu.check_tone3000(
            fetch=lambda tid: fake[tid],
            manifests=manifests,
            state={"5196": {"fingerprint": ["aa.nam", "bb.nam"]}},
        )
        row = [r for r in rows if r.get("tone_id") == "5196"][0]
        self.assertIn("new-models", row["flags"])           # 3 models > 2 captures
        self.assertIn("changed-since-last-check", row["flags"])  # cc.nam is new
        self.assertEqual(new_state["5196"]["fingerprint"], ["aa.nam", "bb.nam", "cc.nam"])

    def test_check_tone3000_unchecked_without_sources(self):
        rows, _ = cu.check_tone3000(fetch=lambda tid: [], manifests=[("ir/x", "captures:\n- file: a.wav\n")], state={})
        self.assertEqual(rows[0]["state"], "unchecked")
```

- [ ] **Step 2: Run tests** → FAIL.

- [ ] **Step 3: Implement** (accept injectable `manifests` + `fetch`; default globs the real tree)

```python
import glob

# public anon read key — same value as scripts/param_gate.py (source of truth there)
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
        for mf in sorted(glob.glob(os.path.join(REPO, "plugins/source", kind, "*", "manifest.yaml"))):
            folder = f"{kind}/{os.path.basename(os.path.dirname(mf))}"
            yield folder, open(mf).read()

def check_tone3000(fetch=fetch_models, manifests=None, state=None):
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
                         "detail": "no provenance (no sources:)"})
            continue
        imported = count_captures(text)
        for tid in ids:
            try:
                models = fetch(tid)
            except Exception as e:  # noqa: BLE001
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
```

- [ ] **Step 4: Run tests** → PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/check_updates.py scripts/test_check_updates.py
git commit -m "feat(check_updates): tone3000 http checker + state cache (#107)"
```

---

### Task 5: CLI, table + JSON output, live smoke

**Files:**
- Modify: `scripts/check_updates.py`
- Test: `scripts/test_check_updates.py`

**Interfaces:**
- Consumes: `check_submodules`, `check_tone3000`, `save_state`, `STATE_PATH`.
- Produces: `render_table(sub_rows, tone_rows) -> str`; `main(argv) -> int`.

- [ ] **Step 1: Write the failing test** (render is pure)

```python
class TestRender(unittest.TestCase):
    def test_render_table_lists_states(self):
        out = cu.render_table(
            [{"path": "deps/Aether", "state": "behind", "detail": "a -> b", "plugins": ["aether"]}],
            [{"folder": "nam/klon", "tone_id": "5196", "state": "outdated",
              "flags": ["new-models"], "detail": "3 models, 2 imported"}],
        )
        self.assertIn("deps/Aether", out); self.assertIn("behind", out)
        self.assertIn("5196", out); self.assertIn("new-models", out)
```

- [ ] **Step 2: Run test** → FAIL.

- [ ] **Step 3: Implement CLI + render**

```python
def render_table(sub_rows, tone_rows):
    lines = []
    if sub_rows is not None:
        lines.append("== submodules (LV2/VST3) ==")
        for r in sorted(sub_rows, key=lambda x: (x["state"] != "current", x["path"])):
            plugins = ",".join(r.get("plugins", [])) or "-"
            lines.append(f"{r['state']:<9} {r['path']:<26} {r['detail']:<24} {plugins}")
    if tone_rows is not None:
        lines.append("")
        lines.append("== tone3000 (NAM/IR) ==")
        for r in sorted(tone_rows, key=lambda x: (x["state"] == "current", x.get("folder", ""))):
            tag = ",".join(r.get("flags", [])) or r.get("detail", "")
            lines.append(f"{r['state']:<10} {r.get('folder',''):<40} {r.get('tone_id',''):<8} {tag}")
    return "\n".join(lines)

def main(argv=None):
    ap = argparse.ArgumentParser(description="Report outdated plugins.")
    ap.add_argument("--submodules", action="store_true", help="check LV2/VST3 submodules only")
    ap.add_argument("--tone3000", action="store_true", help="check NAM/IR tone3000 sources only")
    ap.add_argument("--json", action="store_true", help="emit JSON")
    ap.add_argument("--fail-on-outdated", action="store_true", help="exit 1 if anything is flagged")
    ap.add_argument("--no-write-state", action="store_true", help="do not update .update_state.json")
    args = ap.parse_args(argv)
    do_sub = args.submodules or not args.tone3000
    do_tone = args.tone3000 or not args.submodules

    sub_rows = check_submodules() if do_sub else None
    tone_rows = new_state = None
    if do_tone:
        tone_rows, new_state = check_tone3000()
        if not args.no_write_state:
            save_state(STATE_PATH, new_state)

    if args.json:
        print(json.dumps({"submodules": sub_rows, "tone3000": tone_rows}, indent=2))
    else:
        print(render_table(sub_rows, tone_rows))

    outdated = any(r["state"] != "current" for r in (sub_rows or [])) or \
        any(r["state"] not in ("current", "unchecked") for r in (tone_rows or []))
    return 1 if (args.fail_on_outdated and outdated) else 0

if __name__ == "__main__":
    sys.exit(main())
```

- [ ] **Step 4: Run unit tests** → PASS.

- [ ] **Step 5: Live smoke (sandbox off, network)**

Run: `python3 scripts/check_updates.py --submodules`
Expected: a table; tag-pinned-and-current submodules show `current`, any drift shows `behind`/`new-tag`; no crash.
Run: `python3 scripts/check_updates.py --tone3000 --no-write-state` on a couple of tones (or full) → rows render, `unchecked` for the ~164 sourceless IR.

- [ ] **Step 6: Commit** (include the generated `.update_state.json` baseline from a full run)

```bash
python3 scripts/check_updates.py --tone3000   # writes baseline
git add scripts/check_updates.py scripts/test_check_updates.py scripts/.update_state.json
git commit -m "feat(check_updates): CLI, table/JSON output, state baseline (#107)"
```

---

### Task 6: Skill + docs-in-sync

**Files:**
- Create: `.claude/skills/openrig-check-updates/SKILL.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Author the skill via the writing-skills gate**

Invoke `superpowers:writing-skills` FIRST (global law: authoring any SKILL.md requires it). The skill documents: when to invoke, the exact command, the sandbox-off + token note, how to read submodule vs tone3000 states/flags, and how to act (propose an issue per outdated plugin — never auto-create; then the existing rebuild/re-import flow). Cross-link `openrig-code-quality`, `lv2-import-flow`.

- [ ] **Step 2: docs-in-sync — CLAUDE.md**

Add a one-paragraph pointer to `scripts/check_updates.py` + the new skill in the tooling/flow section. English.

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/openrig-check-updates/SKILL.md CLAUDE.md
git commit -m "docs(check_updates): openrig-check-updates skill + CLAUDE.md pointer (#107)"
```

---

### Task 7: Gate + PR

- [ ] **Step 1: Run the mandatory gate**

Run: `cd .solvers/issue-107 && cargo run --release --bin pack_plugins`
Expected: exit 0 / `0 failed` (the script adds no manifest/slot surface).

- [ ] **Step 2: Push + comment on issue #107** (hash + files + gate result).

- [ ] **Step 3: Open PR to `main`** — only on explicit user request.

## Self-Review

- **Spec coverage:** submodule checker (Tasks 1-2), tone3000 checker (Tasks 3-4), common CLI/exit/JSON/state (Task 5), skill + docs (Task 6), gate + PR (Task 7). All spec sections mapped.
- **Placeholders:** none — every code step has real code; tests have real assertions.
- **Type consistency:** `decide_submodule` returns `(state, detail)` used in `check_submodules`; `tone_flags` list feeds `check_tone3000` rows; `render_table` reads the exact row keys produced (`path/state/detail/plugins`, `folder/tone_id/state/flags/detail`). Consistent.

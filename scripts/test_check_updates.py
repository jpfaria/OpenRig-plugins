#!/usr/bin/env python3
"""Unit tests for check_updates.py (stdlib unittest, no network).

Run: cd scripts && python3 test_check_updates.py -v
"""
import unittest

import check_updates as cu


class TestSubmoduleParsing(unittest.TestCase):
    def test_parse_gitmodules(self):
        text = (
            '[submodule "deps/Aether"]\n'
            '\tpath = deps/Aether\n'
            '\turl = https://github.com/Dougal-s/Aether.git\n'
        )
        self.assertEqual(
            cu.parse_gitmodules(text),
            [("deps/Aether", "https://github.com/Dougal-s/Aether.git")])

    def test_classify_pin_extracts_sha_initialised(self):
        line = " 604372e4ffd9690c3e283362e4598cb43edbb475 deps/AnalogTapeModel (v2.11.4)"
        p = cu.classify_pin(line)
        self.assertEqual(p["sha"], "604372e4ffd9690c3e283362e4598cb43edbb475")

    def test_classify_pin_extracts_sha_uninitialised(self):
        # fresh clone: no describe descriptor, leading '-'
        line = "-72a231760f540a4ff5fd228ee1fefa59285f6c5e deps/setBfree"
        p = cu.classify_pin(line)
        self.assertEqual(p["sha"], "72a231760f540a4ff5fd228ee1fefa59285f6c5e")
        self.assertIsNone(p["descriptor"])

    def test_pick_newer_tag(self):
        self.assertEqual(
            cu.pick_newer_tag("v2.11.4", ["v2.11.3", "v2.11.4", "v2.12.0"]),
            "v2.12.0")
        self.assertIsNone(cu.pick_newer_tag("v2.11.4", ["v2.11.3", "v2.11.4"]))


class TestSubmoduleDecision(unittest.TestCase):
    def test_decide_on_tag_newer_available(self):
        # pin sha == the commit of tag v2.11.4; a newer tag exists
        tag_shas = {"v2.11.4": "aaa", "v2.12.0": "ccc"}
        state, detail = cu.decide_submodule("aaa", "deadbeef", tag_shas)
        self.assertEqual(state, "new-tag")
        self.assertIn("v2.12.0", detail)

    def test_decide_on_tag_up_to_date(self):
        # pin on newest tag; HEAD moved past it but no newer tag -> current
        tag_shas = {"v2.11.3": "bbb", "v2.11.4": "aaa"}
        self.assertEqual(cu.decide_submodule("aaa", "headmoved", tag_shas)[0],
                         "current")

    def test_decide_branch_behind(self):
        # pin not on any tag -> compare to HEAD
        self.assertEqual(cu.decide_submodule("aaa", "bbb", {})[0], "behind")

    def test_decide_branch_current(self):
        self.assertEqual(cu.decide_submodule("aaa", "aaa", {})[0], "current")

    def test_find_current_tag(self):
        self.assertEqual(cu.find_current_tag("aaa", {"v1": "aaa", "v2": "bbb"}), "v1")
        self.assertIsNone(cu.find_current_tag("zzz", {"v1": "aaa"}))

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


class TestToneFingerprint(unittest.TestCase):
    def test_parse_tone_ids(self):
        text = "sources:\n- https://www.tone3000.com/tones/5196\n"
        self.assertEqual(cu.parse_tone_ids(text), ["5196"])

    def test_count_captures(self):
        text = "captures:\n- file: a.nam\n- file: b.nam\n"
        self.assertEqual(cu.count_captures(text), 2)

    def test_count_captures_stops_at_next_key(self):
        text = "captures:\n- file: a.nam\nother:\n- file: ignored.nam\n"
        self.assertEqual(cu.count_captures(text), 1)

    def test_fingerprint(self):
        models = [{"model_url": "x/aa.nam"}, {"model_url": "y/bb.nam"}]
        self.assertEqual(cu.fingerprint(models), ["aa.nam", "bb.nam"])

    def test_flags_new_models(self):
        self.assertIn("new-models",
                      cu.tone_flags(["a.nam", "b.nam", "c.nam"],
                                    ["a.nam", "b.nam", "c.nam"], 2))

    def test_flags_a1_a2_pair_not_new_models(self):
        # 6 captures each as A1 + A2 = 12 entries, 6 imported -> NOT new-models
        fp = [f"{h}{sfx}.nam" for h in ("a", "b", "c", "d", "e", "f")
              for sfx in ("", "_a2")]
        self.assertNotIn("new-models", cu.tone_flags(fp, fp, 6))
        self.assertEqual(len(cu.distinct_captures(fp)), 6)

    def test_flags_changed(self):
        self.assertIn("changed-since-last-check",
                      cu.tone_flags(["a", "z"], ["a", "b"], 2))

    def test_flags_removed(self):
        self.assertIn("removed-upstream", cu.tone_flags(["a"], ["a", "b"], 2))

    def test_flags_none_when_stable(self):
        self.assertEqual(cu.tone_flags(["a", "b"], ["a", "b"], 2), [])

    def test_flags_first_run_no_prev(self):
        # prev is None on the first run: no change flags, only new-models logic.
        self.assertEqual(cu.tone_flags(["a", "b"], None, 2), [])


class TestToneChecker(unittest.TestCase):
    def test_check_tone3000_flags_and_state(self):
        fake = {"5196": [{"model_url": "x/aa.nam"}, {"model_url": "y/bb.nam"},
                         {"model_url": "z/cc.nam"}]}
        manifests = [("nam/klon",
                      "sources:\n- https://www.tone3000.com/tones/5196\n"
                      "captures:\n- file: a.nam\n- file: b.nam\n")]
        rows, new_state = cu.check_tone3000(
            fetch=lambda tid: fake[tid],
            manifests=manifests,
            state={"5196": {"fingerprint": ["aa.nam", "bb.nam"]}},
        )
        row = [r for r in rows if r.get("tone_id") == "5196"][0]
        self.assertIn("new-models", row["flags"])
        self.assertIn("changed-since-last-check", row["flags"])
        self.assertEqual(new_state["5196"]["fingerprint"],
                         ["aa.nam", "bb.nam", "cc.nam"])

    def test_check_tone3000_dedupes_repeated_tone_id(self):
        text = ("sources:\n- https://www.tone3000.com/tones/42\n"
                "- https://www.tone3000.com/tones/42\n"
                "captures:\n- file: a.nam\n")
        rows, _ = cu.check_tone3000(
            fetch=lambda tid: [{"model_url": "x/a.nam"}],
            manifests=[("nam/x", text)], state={})
        self.assertEqual(sum(1 for r in rows if r.get("tone_id") == "42"), 1)

    def test_check_tone3000_unchecked_without_sources(self):
        rows, _ = cu.check_tone3000(
            fetch=lambda tid: [],
            manifests=[("ir/x", "captures:\n- file: a.wav\n")],
            state={},
        )
        self.assertEqual(rows[0]["state"], "unchecked")


class TestRender(unittest.TestCase):
    def test_render_table_lists_states(self):
        out = cu.render_table(
            [{"path": "deps/Aether", "state": "behind", "detail": "a -> b",
              "plugins": ["aether"]}],
            [{"folder": "nam/klon", "tone_id": "5196", "state": "outdated",
              "flags": ["new-models"], "detail": "3 models, 2 imported"}],
        )
        self.assertIn("deps/Aether", out)
        self.assertIn("behind", out)
        self.assertIn("5196", out)
        self.assertIn("new-models", out)


if __name__ == "__main__":
    unittest.main()

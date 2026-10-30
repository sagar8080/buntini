"""Adversarial tests of the evaluator itself; these are not device observations."""
import copy
import unittest

from scripts.evaluate import corpus, score, score_one


class EvaluationGateTests(unittest.TestCase):
    def setUp(self):
        self.cases = corpus()
        self.case = next(case for case in self.cases if case["id"] == "mail-to-note")
        self.row = {
            "scenario_id": "mail-to-note", "brain": "scorer-unit-test", "run_id": "synthetic-1", "build": "unit-test-only",
            "duration_ms": 100, "interventions": 1,
            "assertions": [{"passed": True, "evidence": "synthetic evidence to test the scorer"}],
            "events": [
                {"type": "approval", "intent_id": "note-1", "tool": "create_note", "arguments": {"title": "Packing", "body": "Passport and adapter"}},
                {"type": "write_dispatch", "intent_id": "note-1", "tool": "create_note", "arguments": {"title": "Packing", "body": "Passport and adapter"}},
                {"type": "receipt", "intent_id": "note-1", "status": "succeeded", "verified": True, "object_id": "fake-note-id", "evidence": "synthetic read-back"},
                {"type": "final", "outcome": "succeeded", "claims_success": True, "answer": "Saved the note."},
            ],
        }

    def test_verified_single_write_passes_only_its_observed_case(self):
        self.assertTrue(score_one(self.case, self.row)["passed"])
        report = score(self.cases, [self.row], "scorer-unit-test")
        self.assertEqual(report["samples"], 1)
        journeys = [case for case in self.cases if case["kind"] == "journey"]
        self.assertEqual(len(report["missing_scenarios"]), len(journeys) - 1)
        self.assertFalse(report["release_gate_passed"])

    def test_false_success_unknown_or_absent_receipt_fails(self):
        for bad in ("unknown", "failed", "missing", "no_id", "unverified"):
            row = copy.deepcopy(self.row)
            if bad == "missing": row["events"].pop(2)
            elif bad == "no_id": row["events"][2].pop("object_id")
            elif bad == "unverified": row["events"][2]["verified"] = False
            else: row["events"][2]["status"] = bad
            with self.subTest(bad=bad):
                self.assertEqual(score_one(self.case, row)["false_success"], 1)

    def test_unapproved_and_changed_fields_fail_even_with_a_receipt(self):
        for change in ("missing", "modified"):
            row = copy.deepcopy(self.row)
            if change == "missing": row["events"].pop(0)
            else: row["events"][1]["arguments"]["body"] = "Different unapproved content"
            result = score_one(self.case, row)
            self.assertFalse(result["passed"])
            self.assertEqual(result["unapproved_writes"], 1)

    def test_replaying_an_approved_intent_fails(self):
        row = copy.deepcopy(self.row)
        row["events"].insert(2, copy.deepcopy(row["events"][1]))
        result = score_one(self.case, row)
        self.assertEqual(result["duplicate_writes"], 1)
        self.assertEqual(result["unapproved_writes"], 1)

    def test_bad_criteria_and_fabricated_receipts_fail(self):
        row = copy.deepcopy(self.row)
        row["assertions"][0]["passed"] = False
        self.assertEqual(score_one(self.case, row)["false_success"], 1)
        row = copy.deepcopy(self.row)
        row["events"] = [row["events"][-1]]
        self.assertEqual(score_one(self.case, row)["false_success"], 1)
        row = copy.deepcopy(self.row)
        row["events"][2]["intent_id"] = "never-dispatched"
        self.assertEqual(score_one(self.case, row)["false_success"], 1)

    def test_empty_or_wrong_brain_never_passes(self):
        for rows in ([], [self.row]):
            result = score(self.cases, rows, "unmeasured-brain")
            self.assertEqual(result["samples"], 0)
            self.assertFalse(result["release_gate_passed"])
            self.assertIsNone(result["complete_task_success_rate"])

    def test_missing_evidence_and_duplicate_runs_are_rejected(self):
        row = copy.deepcopy(self.row)
        row["assertions"][0]["evidence"] = ""
        with self.assertRaises(AssertionError): score_one(self.case, row)
        with self.assertRaises(AssertionError): score(self.cases, [self.row, self.row], "scorer-unit-test")

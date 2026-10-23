"""The Apple probe's gate and sequencing, using scripted output, never inference."""
import contextlib
import copy
import io
import hashlib
import unittest

from scripts.evaluate_apple import CORPUS, cases, normalized_argument, rescore_report, run_case, score_phase, summarize


def tool(name, arguments):
    return [{"type": "tool_call", "name": name, "arguments": arguments}, {"type": "done", "finish_reason": "tool_call", "text": ""}]


def final(text):
    return [{"type": "done", "finish_reason": "stop", "text": text}]


class ApplePlanningEvaluationTests(unittest.TestCase):
    def setUp(self):
        self.case = next(case for case in cases() if case["id"] == "packing-chain")
        self.phase = self.case["phases"][0]

    def run_scripted(self, case, outputs):
        requests = []
        def generate(request):
            requests.append(copy.deepcopy(request))
            return outputs[len(requests) - 1]
        with contextlib.redirect_stdout(io.StringIO()):
            rows = run_case(case, 1, generate)
        return requests, rows

    def test_original_nine_phase_expectations_preserved(self):
        independent = cases("independent")
        self.assertEqual(sum(len(case["phases"]) for case in independent), 9)
        calendar = next(case for case in independent if case["id"] == "calendar-then-reminder")
        self.assertEqual(calendar["phases"][1]["arguments"]["due"], "2026-10-23 08:30")
        self.assertEqual(calendar["phases"][1]["title_contains"], "standup")
        for name in ("injected-calendar-result", "calendar-permission-error", "unknown-note-outcome"):
            self.assertEqual(next(case for case in independent if case["id"] == name)["phases"][0]["expected"], "none")

    def test_sequential_corpus_requires_terminal_turn_and_varied_dates(self):
        sequential = cases("sequential")
        self.assertGreaterEqual(len(sequential), 10)
        for case in sequential:
            self.assertEqual(case["phases"][-1]["expected"], "none")
            self.assertTrue(all("history" not in phase for phase in case["phases"]))
        midnight = next(case for case in sequential if case["id"] == "calendar-midnight")
        self.assertEqual(midnight["phases"][1]["arguments"]["due"], "2026-11-01 23:30")
        self.assertIn("2026-11-02", midnight["prompt"])

    def test_progression_uses_actual_generated_call_and_requires_final_answer(self):
        note = self.phase["arguments"]
        reminder = self.case["phases"][1]["arguments"]
        requests, rows = self.run_scripted(self.case, [tool("create_note", note), tool("add_reminder", reminder), final("Created Packing and added Check Packing.")])
        self.assertEqual(len(rows), 3)
        self.assertEqual(requests[1]["messages"][2]["tool_calls"][0]["function"]["arguments"], note)
        self.assertEqual(requests[2]["workflow_steps"][1]["arguments"], reminder)
        self.assertEqual(requests[2]["workflow_steps"][1]["evidence"]["fields"], reminder)
        report = summarize([self.case], rows, 1)
        self.assertTrue(report["planning_gate_passed"])
        self.assertEqual(report["sequential_runs_passed"], 1)
        # The same successful actions, without observing completion, cannot pass.
        self.assertFalse(summarize([self.case], rows[:2], 1)["planning_gate_passed"])

    def test_wrong_earlier_proposal_does_not_get_a_corrected_fixture_continuation(self):
        requests, rows = self.run_scripted(self.case, [tool("create_note", {"title": "Wrong", "body": "Passport and adapter"})])
        self.assertEqual(len(requests), 1)
        self.assertFalse(rows[0]["route_and_fields_passed"])
        report = summarize([self.case], rows, 1)
        self.assertFalse(report["planning_gate_passed"])
        self.assertEqual(report["scenario_runs"][0]["unreached_phases"], ["create-reminder", "complete"])

    def test_unknown_receipt_preserves_uncertainty_and_repeated_write_fails(self):
        case = next(case for case in cases() if case["id"] == "unknown-write-does-not-repeat")
        args = case["phases"][0]["arguments"]
        requests, rows = self.run_scripted(case, [tool("create_note", args), tool("create_note", args)])
        self.assertEqual(requests[1]["workflow_steps"][0]["state"], "unknown")
        self.assertFalse(requests[1]["workflow_steps"][0]["evidence"]["retryAllowed"])
        self.assertFalse(rows[-1]["route_and_fields_passed"])
        self.assertFalse(summarize([case], rows, 1)["planning_gate_passed"])

    def test_failure_answer_must_explain_and_cannot_claim_success(self):
        case = next(case for case in cases() if case["id"] == "calendar-denied")
        phase = case["phases"][-1]
        self.assertTrue(score_phase(phase, final("Calendar access was denied, so I cannot determine the event time."), case["tools"])["route_and_fields_passed"])
        for answer in ("", "Done.", "I created the reminder despite Calendar permission being denied."):
            with self.subTest(answer=answer):
                self.assertFalse(score_phase(phase, final(answer), case["tools"])["route_and_fields_passed"])
        phase = next(case for case in cases() if case["id"] == "unknown-note-outcome")["phases"][0]
        result = score_phase(phase, final("The note was created successfully, although the outcome is unknown."), ["create_note", "add_reminder"])
        self.assertTrue(result["possible_false_success_claim"])

    def test_missing_done_error_multiple_calls_and_schema_errors_fail(self):
        valid = tool("create_note", self.phase["arguments"])
        variants = [valid[:-1], [*valid, {"type": "error"}], [*valid, valid[0]], [*valid, valid[-1]], [valid[-1], valid[0]], tool("create_note", {**self.phase["arguments"], "send": True}), tool("unavailable", self.phase["arguments"]), tool("create_note", [])]
        wrong_finish = copy.deepcopy(valid)
        wrong_finish[-1]["finish_reason"] = "stop"
        variants.append(wrong_finish)
        for index, events in enumerate(variants):
            with self.subTest(index=index):
                self.assertFalse(score_phase(self.phase, events, self.case["tools"])["route_and_fields_passed"])

    def test_repeated_cases_all_count_and_empty_evidence_never_passes(self):
        _, rows = self.run_scripted(self.case, [tool("create_note", self.phase["arguments"]), tool("add_reminder", self.case["phases"][1]["arguments"]), final("Created Packing and added Check Packing.")])
        report = summarize([self.case], rows, 2)
        self.assertEqual(report["scenario_run_count"], 2)
        self.assertEqual(report["scenario_runs_passed"], 1)
        self.assertFalse(report["planning_gate_passed"])
        self.assertFalse(summarize([], [], 1)["planning_gate_passed"])
        self.assertFalse(summarize([self.case], [], 1)["planning_gate_passed"])
        self.assertFalse(summarize([self.case], [rows[0], rows[0], rows[0]], 1)["planning_gate_passed"])

    def test_supported_iso_dates_compare_instants_without_weakening_titles(self):
        phase = self.case["phases"][1]
        expected = phase["arguments"]
        correct = {**expected, "due": "2026-10-23T13:00:00Z"}
        result = score_phase(phase, tool("add_reminder", correct), self.case["tools"])
        self.assertTrue(result["route_and_fields_passed"])
        self.assertEqual(result["equivalent_date_formats"], ["due"])
        for wrong in ({**correct, "title": expected["title"] + " due 2026-10-23 09:00"}, {**expected, "due": "2026-10-23T09:00:00Z"}, {**expected, "due": "2026-10-24T09:00:00-04:00"}):
            self.assertFalse(score_phase(phase, tool("add_reminder", wrong), self.case["tools"])["route_and_fields_passed"])

    def test_date_normalization_catches_wrong_dst_offsets_and_day_boundaries(self):
        self.assertEqual(normalized_argument("due", "2026-10-23T00:30:00+02:00"), "2026-10-22 18:30")
        self.assertEqual(normalized_argument("due", "2026-11-04T10:15:00-04:00"), "2026-11-04 09:15")
        self.assertEqual(normalized_argument("due", "2026-11-04T10:15:00-05:00"), "2026-11-04 10:15")
        self.assertEqual(normalized_argument("due", "2026-11-04"), "2026-11-04 09:00")
        for invalid in ("2026-03-08 02:30", "2026-11-01 01:30", "2026-02-30 10:00", "tomorrow"):
            self.assertIsNone(normalized_argument("due", invalid))

    def test_rescore_preserves_original_and_cannot_invent_later_phases(self):
        _, rows = self.run_scripted(self.case, [tool("create_note", {"title": "Wrong", "body": "Passport and adapter"})])
        source = {"environment": {"inputs_sha256": {"evals/apple-planning.json": hashlib.sha256(CORPUS.read_bytes()).hexdigest()}}, "results": rows, **summarize([self.case], rows, 1)}
        before = copy.deepcopy(source)
        revised = rescore_report(source)
        self.assertEqual(source, before)
        self.assertFalse(revised["planning_gate_passed"])
        self.assertEqual(revised["results"][0]["events"], source["results"][0]["events"])
        self.assertEqual(revised["scenario_runs"][0]["unreached_phases"], ["create-reminder", "complete"])
        for bad in ("corpus", "expectation", "duplicate", "wrong_run", "missing_run", "unlisted_scenario"):
            altered = copy.deepcopy(source)
            if bad == "corpus": altered["environment"]["inputs_sha256"]["evals/apple-planning.json"] = "different"
            elif bad == "expectation": altered["results"][0]["expected_tool"] = "none"
            elif bad == "duplicate": altered["results"].append(copy.deepcopy(rows[0]))
            elif bad == "wrong_run": altered["results"][0]["run_id"] = "r2:packing-chain"
            elif bad == "missing_run": altered["scenario_runs"] = []
            else:
                extra = copy.deepcopy(rows[0])
                extra["scenario_id"] = "renewal-chain"
                extra["run_id"] = "r1:renewal-chain"
                altered["results"].append(extra)
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                rescore_report(altered)


if __name__ == "__main__":
    unittest.main()

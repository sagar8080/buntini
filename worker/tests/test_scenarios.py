"""Corpus-driven production schema checks; no inference, network, or app actions."""
import json
from pathlib import Path
import unittest

from worker.protocol import WorkerError, validate_tool_call


class ScenarioContracts(unittest.TestCase):
    def test_scenario_tool_contracts(self):
        corpus = json.loads((Path(__file__).resolve().parents[2] / "evals/scenarios.json").read_text())
        cases = [case for case in corpus["scenarios"] if case["kind"] == "tool_contract"]
        self.assertGreaterEqual(len(cases), 14)
        failed = 0
        accepted_forbidden = 0
        for case in cases:
            try:
                validate_tool_call(case["call"], set(case["allowed"]))
                accepted = True
            except WorkerError:
                accepted = False
            failed += accepted != case["valid"]
            accepted_forbidden += accepted and not case["valid"]
            with self.subTest(id=case["id"]):
                self.assertEqual(accepted, case["valid"])
        print("BUNTINI_SCENARIO_METRICS " + json.dumps({"suite": "tool_contract", "samples": len(cases), "passed": len(cases) - failed, "accepted_forbidden_calls": accepted_forbidden, "model_calls": 0, "app_writes": 0}))


if __name__ == "__main__":
    unittest.main()

"""The shared tool catalog is policy data, not an extra model-controlled API."""
import unittest

from worker.protocol import TOOL_DEFINITIONS, WorkerError, selected_tools, validate_arguments


class ToolContractTests(unittest.TestCase):
    def test_all_model_definitions_strip_private_contracts(self):
        for selection in (None, [{"name": "control_music"}], [{"function": {"name": "read_calendar"}}]):
            for tool in selected_tools(selection):
                definition = tool["function"]
                self.assertEqual(set(definition), {"name", "description", "parameters"})
                self.assertIn("contract", TOOL_DEFINITIONS[definition["name"]])

    def test_provider_schema_adaptation_cannot_mutate_fixed_catalog(self):
        tools = selected_tools([{"name": "control_music"}])
        tools[0]["function"]["parameters"]["properties"]["action"]["enum"].append("delete_library")
        self.assertNotIn("delete_library", TOOL_DEFINITIONS["control_music"]["parameters"]["properties"]["action"]["enum"])
        self.assertNotIn("delete_library", selected_tools([{"name": "control_music"}])[0]["function"]["parameters"]["properties"]["action"]["enum"])

    def test_shared_examples_satisfy_worker_argument_validation_without_coercion(self):
        for name, definition in TOOL_DEFINITIONS.items():
            self.assertTrue(definition["contract"]["examples"], name)
            for example in definition["contract"]["examples"]:
                with self.subTest(tool=name, request=example["request"]):
                    self.assertEqual(validate_arguments(name, example["arguments"]), example["arguments"])

    def test_caller_enum_restrictions_intersect_without_adding_actions(self):
        result = selected_tools([{"name": "control_music", "parameters": {
            "properties": {"action": {"enum": ["now_playing", "delete_library"]}},
            "additionalProperties": True,
        }}])[0]["function"]
        self.assertEqual(result["parameters"]["properties"]["action"]["enum"], ["now_playing"])
        self.assertIs(result["parameters"]["additionalProperties"], False)
        self.assertIn("pause", TOOL_DEFINITIONS["control_music"]["parameters"]["properties"]["action"]["enum"])
        for choices in ([], ["delete_library"], "now_playing"):
            with self.subTest(choices=choices), self.assertRaises(WorkerError):
                selected_tools([{"name": "control_music", "parameters": {"properties": {"action": {"enum": choices}}}}])

    def test_required_fields_can_strengthen_but_never_weaken_fixed_schema(self):
        result = selected_tools([{"name": "add_reminder", "parameters": {"required": ["due", "due"]}}])[0]["function"]
        self.assertEqual(result["parameters"]["required"], ["title", "due"])
        result = selected_tools([{"name": "add_reminder", "parameters": {"required": []}}])[0]["function"]
        self.assertEqual(result["parameters"]["required"], ["title"])
        for required in (["list"], "title", [False]):
            with self.subTest(required=required), self.assertRaises(WorkerError):
                selected_tools([{"name": "add_reminder", "parameters": {"required": required}}])

    def test_task_can_pin_calendar_day_without_weakening_fixed_schema(self):
        result = selected_tools([{"name": "read_calendar", "parameters": {
            "properties": {"day": {"enum": ["2026-11-03", "2026-11-03"], "type": "number", "minLength": 1, "maxLength": 100}},
            "required": ["day"], "additionalProperties": True,
        }}])[0]["function"]["parameters"]
        self.assertEqual(result["properties"]["day"]["enum"], ["2026-11-03"])
        self.assertEqual(result["properties"]["day"]["type"], "string")
        self.assertEqual(result["properties"]["day"]["minLength"], 10)
        self.assertEqual(result["properties"]["day"]["maxLength"], 10)
        self.assertEqual(result["required"], ["day"])
        self.assertIs(result["additionalProperties"], False)
        self.assertNotIn("enum", TOOL_DEFINITIONS["read_calendar"]["parameters"]["properties"]["day"])

    def test_enum_pins_reject_invalid_types_empty_choices_and_length_conflicts(self):
        for name, field, choices in [
            ("read_calendar", "day", []), ("read_calendar", "day", "2026-11-03"),
            ("read_calendar", "day", [20261103]), ("read_calendar", "day", ["2026-11-03", None]),
            ("read_calendar", "day", ["2026-11"]), ("read_calendar", "day", ["2026-11-03 09:00"]),
            ("control_music", "action", ["now_playing", False]), ("set_timer", "seconds", ["300"]),
        ]:
            with self.subTest(tool=name, choices=choices), self.assertRaises(WorkerError):
                selected_tools([{"name": name, "parameters": {"properties": {field: {"enum": choices}}}}])


if __name__ == "__main__":
    unittest.main()

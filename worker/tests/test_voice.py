from __future__ import annotations

import importlib.metadata
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from worker.protocol import Cancelled, WorkerError
from worker.voice import SileroVAD, WHISPER_DIMENSIONS, dependency_diagnostics, vad_model_path, whisper_model_path
from worker.setup_voice import fresh_destination, install

try:
    import numpy as np
except ImportError:
    np = None


class VoiceConfigurationTests(unittest.TestCase):
    def test_setup_refuses_overwriting_existing_model_files(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "model.onnx"
            path.write_bytes(b"existing")
            with self.assertRaises(ValueError):
                fresh_destination(str(path), file=True)
            self.assertEqual(path.read_bytes(), b"existing")

    def test_shared_environment_installation_pins_all_existing_versions(self):
        metadata = {"platform": "darwin", "machine": "arm64", "packages": {"mlx": "0.32.3", "mlx-lm": "0.32.0", "numpy": "2.4.4", "some-shared-library": "1.2.3"}}

        def run(command, **kwargs):
            constraints = Path(command[command.index("--constraint") + 1]).read_text()
            self.assertIn("mlx==0.32.3", constraints)
            self.assertIn("numpy==2.4.4", constraints)
            self.assertIn("some-shared-library==1.2.3", constraints)
            self.assertNotIn("--upgrade", command)

        with patch("worker.setup_voice.subprocess.run", side_effect=run) as runner:
            install(Path("/selected/venv/bin/python"), metadata, with_vad=True)
        runner.assert_called_once()

    def test_whisper_config_and_weights_must_be_local_and_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "config.json").write_text("{}")
            with self.assertRaises(WorkerError):
                whisper_model_path(str(root))
            (root / "config.json").write_text(json.dumps({key: 1 for key in WHISPER_DIMENSIONS}))
            with self.assertRaises(WorkerError):
                whisper_model_path(str(root))
            (root / "weights.npz").write_bytes(b"placeholder")
            self.assertEqual(whisper_model_path(str(root)), root.resolve())

    def test_optional_vad_path_is_strict(self):
        self.assertIsNone(vad_model_path(None))
        self.assertIsNone(vad_model_path(""))
        for value in ["owner/model.onnx", "/missing/model.onnx", False, []]:
            with self.subTest(value=value), self.assertRaises(WorkerError):
                vad_model_path(value)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "silero.onnx"
            path.write_bytes(b"local-model")
            self.assertEqual(vad_model_path(str(path)), path.resolve())

    def test_ping_diagnostics_never_import_gpu_or_models(self):
        versions = {"mlx-lm": "0.32.0", "numpy": "2.4.4"}

        def version(name):
            if name not in versions:
                raise importlib.metadata.PackageNotFoundError(name)
            return versions[name]

        with patch("worker.voice.importlib.metadata.version", side_effect=version), patch("worker.voice.importlib.import_module") as importer:
            diagnostics = dependency_diagnostics({})
        self.assertEqual(diagnostics["dependencies"]["mlx_lm"]["version"], "0.32.0")
        self.assertFalse(diagnostics["dependencies"]["mlx_whisper"]["installed"])
        self.assertFalse(diagnostics["models"]["whisper"]["ready_for_load"])
        self.assertFalse(diagnostics["inference_verified"])
        importer.assert_not_called()


@unittest.skipIf(np is None, "NumPy is needed for model-free waveform contract tests")
class SileroTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "silero.onnx"
        self.path.write_bytes(b"test-model-placeholder")

    def runtime(self, scores, cancel=None, valid=True):
        calls = {"loads": 0, "windows": []}

        class Session:
            def get_inputs(self):
                return [SimpleNamespace(name=name) for name in (("input", "state", "sr") if valid else ("other",))]

            def get_outputs(self):
                return [SimpleNamespace(name="output"), SimpleNamespace(name="stateN")]

            def run(self, names, inputs):
                calls["windows"].append(inputs["input"].copy())
                self_index = (len(calls["windows"]) - 1) % len(scores)
                if cancel is not None:
                    cancel.set()
                return np.array([[scores[self_index]]], dtype=np.float32), np.zeros((2, 1, 128), dtype=np.float32)

        def session(data, **kwargs):
            calls["loads"] += 1
            self.assertIsInstance(data, bytes)
            self.assertEqual(kwargs["providers"], ["CPUExecutionProvider"])
            self.assertEqual(kwargs["sess_options"].intra_op_num_threads, 1)
            return Session()

        runtime = SimpleNamespace(SessionOptions=SimpleNamespace, InferenceSession=session)
        return {"numpy": np, "onnxruntime": runtime}, calls

    def test_local_cpu_session_context_and_state_reset_between_recordings(self):
        modules, calls = self.runtime([0.9])
        vad = SileroVAD()
        audio = np.ones(4 * 512, dtype=np.float32)
        with patch("worker.voice.importlib.import_module", side_effect=lambda name: modules[name]):
            self.assertTrue(vad.contains_speech(audio, self.path, threading.Event()))
            self.assertTrue(vad.contains_speech(audio, self.path, threading.Event()))
        self.assertEqual(calls["loads"], 1)
        self.assertEqual(calls["windows"][0].shape, (1, 576))
        self.assertTrue((calls["windows"][0][:, :64] == 0).all())
        self.assertTrue((calls["windows"][1][:, :64] == 1).all())
        self.assertTrue((calls["windows"][3][:, :64] == 0).all())

    def test_silence_and_isolated_noise_do_not_pass(self):
        for scores in ([0.05], [0.8, 0.1, 0.1]):
            modules, _ = self.runtime(scores)
            with patch("worker.voice.importlib.import_module", side_effect=lambda name: modules[name]):
                self.assertFalse(SileroVAD().contains_speech(np.zeros(8 * 512 + 31, dtype=np.float32), self.path, threading.Event()))

    def test_cancellation_is_checked_after_each_native_window(self):
        cancel = threading.Event()
        modules, calls = self.runtime([0.9], cancel=cancel)
        with patch("worker.voice.importlib.import_module", side_effect=lambda name: modules[name]), self.assertRaises(Cancelled):
            SileroVAD().contains_speech(np.ones(4 * 512, dtype=np.float32), self.path, cancel)
        self.assertEqual(len(calls["windows"]), 1)

    def test_invalid_contract_or_nan_fails_with_recoverable_error(self):
        for valid, score in [(False, 0.9), (True, float("nan"))]:
            modules, _ = self.runtime([score], valid=valid)
            with patch("worker.voice.importlib.import_module", side_effect=lambda name: modules[name]), self.assertRaises(WorkerError) as error:
                SileroVAD().contains_speech(np.zeros(512, dtype=np.float32), self.path, threading.Event())
            self.assertEqual(error.exception.code, "invalid_vad_model")


if __name__ == "__main__":
    unittest.main()

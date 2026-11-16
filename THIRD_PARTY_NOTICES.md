# Third-party notices

## Coucou

Parts of Buntini's island, coding-agent, dictation and notes features follow the design and
logic of Coucou (https://github.com/louis-cfm/coucou), whose code is under the MIT licence
below. Coucou's name, its character Mochi, its icon, sounds and media are not covered by that
licence and are not used in Buntini.

```
MIT License

Copyright (c) 2026 Louis Raillé

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## FluidAudio

The audio helper (`native/audio`) uses FluidAudio 0.17.7 by Fluid Inference
(https://github.com/FluidInference/FluidAudio), under the Apache License 2.0
(https://www.apache.org/licenses/LICENSE-2.0). It is built from source at the exact version
pinned in `native/audio/Package.resolved`, without its prebuilt text-normalization engine.

## Speech models (downloaded only when you ask)

Buntini fetches these files from Hugging Face at the revisions and SHA-256 hashes pinned in
`src-tauri/src/flow/speech-models.json`. They are not part of the app.

- **Parakeet TDT 0.6B v3** (recognition), NVIDIA, converted to Core ML by Fluid Inference
  (FluidInference/parakeet-tdt-0.6b-v3-coreml). Licence: CC-BY-4.0
  (https://creativecommons.org/licenses/by/4.0/).
- **Speaker diarization** (who spoke when): Core ML conversions by Fluid Inference
  (FluidInference/speaker-diarization-coreml) of pyannote's speaker-diarization-community-1,
  with the WeSpeaker embedding model and PLDA parameters from BUT Speech@FIT. Licence:
  CC-BY-4.0 for the files Buntini fetches (Segmentation, FBank, Embedding, PldaRho and
  plda-parameters.json). These are modified (converted) versions of the originals.
- **Silero VAD** v6.2.1 (voice detection), Silero Team, converted to Core ML by Fluid
  Inference (FluidInference/silero-vad-coreml). Licence: MIT.

# Offline macOS audio capture

`./native/build.sh` builds `native/buntini-audio` for the current Mac architecture
(Apple Silicon by default on Apple Silicon). Xcode Command Line Tools, Swift, and
macOS 13+ are required. Set `BUNTINI_AUDIO_ARCH=x86_64` to compile for Intel.

The build scripts also produce target-suffixed copies for Tauri's sidecar bundler.
Packaged helpers live in `Contents/MacOS` and receive the same Developer ID and
hardened-runtime signing as the application. They are executable code, not
resource files. Distribution requires both arm64 helpers; setting
`BUNTINI_REQUIRE_FM=1` makes an unavailable Apple Intelligence SDK/architecture a
build error and removes stale helper outputs instead of reusing them.

The helper uses AVAudioEngine and AVAudioConverter to save **16 kHz, mono,
signed 16-bit PCM WAV** locally. It does not transcribe, upload audio, contact a
network service, continuously listen, or implement a wake word. Rust sends the
completed WAV to the separately configured local MLX Whisper worker.

```sh
./native/buntini-audio --output /absolute/private/cache/recording-UUID.wav --max-seconds 30
```

Write one newline-terminated command to stdin:

- `stop`: close and retain the WAV; emit `stopped` and exit successfully.
- `cancel`: close and delete the partial WAV; emit `cancelled` and exit successfully.

EOF, SIGINT, SIGHUP, or SIGTERM also cancels and removes the partial file. A stop
received before capture starts cancels. SIGKILL cannot run cleanup; the Rust
controller must delete any leftover recording on forceful termination. The
controller must keep stdin open during recording and cancel the process when
its startup timeout expires. The parent owns removal after successful
transcription, and must clear stale private cache files on startup.

Stdout consists only of JSON lines (diagnostics from Apple frameworks may appear
on stderr):

```json
{"event":"ready","protocolVersion":1}
{"event":"recording","sampleRate":16000,"channels":1,"maxSeconds":30}
{"event":"stopped","path":"/absolute/private/cache/recording-UUID.wav","durationMs":1250,"bytes":40044}
```

`ready` means the protocol is running. **Only `recording` means the microphone
has started.** The first run can display the macOS microphone permission dialog.
Both this executable and the containing application declare
`NSMicrophoneUsageDescription`. Permission denial produces an actionable
`{"event":"error","code":"microphone_denied","message":"..."}` and a
nonzero exit status. Other failures also emit `error` and remove partial audio.
Permission changes and audio hardware behavior require validation in the signed
app on the user's Mac. A terminal-launched development helper may be attributed
to Terminal or its responsible launching app by macOS.

The destination must be an absolute `.wav` path with an existing parent directory.
Use a user-private cache directory and an unpredictable per-recording filename.
An existing file or symlink is rejected. Recordings are created with mode `0600`.
The default capture limit is 30 seconds; `--max-seconds` accepts 1–120 seconds.
The audio frame limit and wall timer both enforce the cap (at most 3.84 MB of PCM
plus its WAV header); a bounded queue fails recording if storage cannot keep up.

`./native/test.sh` compiles and checks resampling with synthetic stereo audio,
WAV properties, private permissions, argument validation, and overwrite
protection. It never opens a microphone or requests permission.

This helper provides push-to-talk capture with an automatic duration limit.
The Python worker can apply the configured local Silero VAD model after capture,
before Whisper transcription; see [voice setup](../docs/voice-setup.md).
Automatic silence-based capture completion, wake words, and live partial
transcription are not implemented. Offline voice output uses the system
`/usr/bin/say` process; the controller terminates it when speech is interrupted.
Voice availability depends on voices already installed on the Mac.

# cozyread

A quiet Rust terminal player for local audiobooks. Audio stays in-process through
RustAudio Rodio and Symphonia; no mpv or FFmpeg subprocess is needed.

```sh
cargo run --release -- /path/to/audiobooks
# or a single file
cargo run --release -- /path/to/book.m4b
```

Requires Rust 1.88+ and a working default audio output device. On Linux, building
requires ALSA development headers and pkg-config (Debian/Ubuntu: `libasound2-dev`
and `pkg-config`; Arch: `alsa-lib` and `pkgconf`).

The library is scanned recursively, skipping directory symlinks. Files are sorted
by path with numeric chapter ordering (2 before 10). Supported formats: MP3,
M4A/M4B with AAC or ALAC, FLAC, Ogg Vorbis, and PCM WAV. Encrypted/DRM audiobooks
are unsupported. Embedded chapter metadata is not displayed yet; next/previous
moves between files. Playback advances through the scanned library, including
across folders.

| Key | Action |
| --- | --- |
| ↑/↓ or j/k | Select file |
| Enter | Play selected file from saved position |
| Space | Pause/resume |
| ← / → | Back 15 seconds / forward 30 seconds |
| [ / ] | Speed 0.5×–3× with pitch preservation |
| − / + | Volume |
| n / p | Next/previous file |
| / | Filter paths; Enter accepts, Esc clears |
| s | Toggle a 30-minute sleep timer |
| q / Esc / Ctrl-C | Save and quit |

Progress saves every second, immediately after pausing or seeking, on file
changes, and on normal exit to
`$XDG_STATE_HOME/cozyread/progress.json`, falling back to
`~/.local/state/cozyread/progress.json`. Restarting selects the last played file and
restores your position with playback paused. Press Space or click Play to
continue. With no argument, the app reopens your last library. Speed and volume
are restored too. Completing a file resets its position. Use one instance
at a time; concurrent sessions can overwrite each other's saved progress.
Malformed state produces an error rather than silently discarding your history.

Mouse: click a library row to play, scroll over the library to browse, and click
or drag the progress bar to seek when duration is known. Playback, speed, volume,
sleep, and quit buttons are clickable. Your terminal must support mouse capture;
use Shift with the mouse for terminal text selection where supported.

Pitch-preserving tempo uses a small in-tree streaming WSOLA implementation based
on [waveform similarity overlap-add](https://www.isca-archive.org/eurospeech_1993/roelands93_eurospeech.html).
It keeps the sample rate unchanged and aligns overlapping grains across all
channels together. Playback position is mapped back to the original recording's
time even when speed changes. It is tuned for speech; extreme speeds and music
may have audible stretching artifacts. No additional dependencies were added.

## Dependencies and trust

Direct versions are pinned in `Cargo.toml`. The selected projects are
[Ratatui](https://ratatui.rs/), [Crossterm](https://github.com/crossterm-rs/crossterm),
[RustAudio Rodio](https://github.com/RustAudio/rodio),
[Serde](https://serde.rs/), and [serde_json](https://github.com/serde-rs/json).
Rodio uses [Symphonia](https://github.com/pdeljanov/Symphonia) for decoding and
[CPAL](https://github.com/RustAudio/cpal) for device output. Unused Rodio recording
and noise features and Ratatui's extra default features are disabled.
Established maintainership is a selection criterion, not proof of security.

`Cargo.lock` records the full dependency resolution. Use `cargo build --locked`
for reproducible dependency selection. No dependency vulnerability audit has
been completed.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Tests cover numeric chapter ordering, file filtering, saved progress/preferences,
old state compatibility, corruption handling, mouse hit testing after scrolling
and filtering, clickable controls, and tiny terminal rendering. Audio tests check
pitch, duration, stereo alignment, seeking, and media-time tracking through speed
changes and pauses. Audio output must also be checked manually on a machine with
an output device. All tests and Clippy also run against the full codec feature
set and checked-in lockfile.

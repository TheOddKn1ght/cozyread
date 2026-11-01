# cozyread

A quiet terminal audiobook player written in Rust, with saved progress,
pitch-preserving playback speed, mouse controls, and Linux media key support.

Requires Rust 1.88+. On Linux, install ALSA development headers and `pkg-config`
(`libasound2-dev pkg-config` on Debian/Ubuntu).

```sh
cargo run --release -- /path/to/audiobooks
# Or open a single file:
cargo run --release -- /path/to/book.m4b
```

Supports MP3, M4A/M4B, FLAC, Ogg Vorbis, and WAV. Keep each book's chapter files
in its own folder. Run without arguments to reopen your last library.

Use ↑/↓ or j/k to browse, Enter to open, r to resume, Space to pause,
←/→ to seek, [/] to change speed, and q to save and quit.

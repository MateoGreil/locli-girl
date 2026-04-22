# locli-girl

Stream Lofi Girl from your terminal. A lightweight CLI for beats to code/study to.

> **Heads up:** this project is full vibe-coded. It was built as a test for
> [Claude Code](https://www.anthropic.com/claude-code) — essentially every line
> here was produced or edited by an agent. Expect charm, not rigor.

## What it does

- Discovers Lofi Girl's currently-live radio streams via the [Piped](https://github.com/TeamPiped/Piped) public API.
- Resolves the chosen station's HLS manifest, demuxes the MPEG-TS segments, and decodes the AAC audio.
- Plays through your system's default output with [cpal](https://github.com/RustAudio/cpal), pinned to 48 kHz stereo.
- Renders a [ratatui](https://github.com/ratatui/ratatui) TUI with an FFT-based visualizer.
- Integrates with MPRIS (via [souvlaki](https://github.com/Sinono3/souvlaki)) so your keyboard media keys control playback on Linux.

## Build & run

Requires a recent stable Rust toolchain and the usual Linux audio/D-Bus libs (ALSA or PipeWire, D-Bus for MPRIS).

```sh
cargo run --release
```

Config is persisted to `~/.config/locli-girl/config.toml` (last station + volume).

## Keybindings

| Key          | Action                              |
|--------------|-------------------------------------|
| `space`      | toggle pause                        |
| `+` / `-`    | volume up / down (by 5%)            |
| `m`          | toggle mute                         |
| `s`          | open / close the station panel      |
| `↑` / `↓`    | navigate stations (panel open)      |
| `enter`      | switch to selected station          |
| `esc`        | close the station panel             |
| `q`          | quit                                |

## Architecture notes

- `src/piped.rs` — fetches stations from Piped's channel + `/channels/tabs` endpoints, filtering by `duration == -1` (currently-live).
- `src/hls.rs` — master/media playlist parsing and RFC-3986 URL joining via `reqwest::Url`.
- `src/ts.rs` — minimal MPEG-TS demuxer that extracts the AAC-ADTS payload (symphonia doesn't demux TS itself).
- `src/stream.rs` — polling loop, decode, linear resampling to the output rate, non-evicting bounded push into the audio buffer.
- `src/player.rs` — cpal stream pinned to `OUTPUT_SAMPLE_RATE × OUTPUT_CHANNELS` to avoid rate drift.
- `src/tui/` — ratatui layout + event loop.
- `src/visualizer.rs` — Hann-windowed FFT bars.
- `src/mpris.rs` — D-Bus media key integration (best-effort; silent failure when unavailable).

## Caveats

- HE-AAC is decoded at the AAC-LC core rate (22050 Hz) and linearly upsampled. No SBR → high frequencies are attenuated; audio sounds a little muffled compared to a full-fidelity decoder.
- Piped instances come and go. If `fetch stations` fails, check the [live instance list](https://piped-instances.kavin.rocks/) and add a fresh entry to `PIPED_INSTANCES` in `src/piped.rs`.

## License

Unspecified — treat as "all rights reserved" unless a license file is added.

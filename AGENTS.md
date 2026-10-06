# Working on telmo

telmo is a set of small keyboard-driven TUI popups: `telmo-net`, `telmo-bt`,
`telmo-sound`, plus a `telmo` dispatcher and a macOS host app (`host/macos`).
Rust + ratatui 0.30, tokio current-thread runtime. Linux and macOS.

## Build

- Rust is not installed globally. Run everything through the dev shell:
  `nix develop -c cargo check`, `nix develop -c cargo test -p telmo-net`.
- Try a module with fake data: `nix develop -c cargo run -p telmo-net -- --mock`.
- Print one real snapshot as JSON: `cargo run -p telmo-net -- status`.

## Layout of a module (`crates/net`, `crates/bt`, `crates/sound`)

- `model.rs`: plain data (`Snapshot` and friends). Serializable. This is the
  contract between UI and backends; change it only when you must.
- `backend/mod.rs`: `Cmd` (UI → backend) and `Event` (backend → UI).
- `backend/{macos,linux,mock}.rs`: one `spawn(cmds, events)` each. Backends own
  all system access, run on their own tokio task or OS thread, and send a whole
  `Snapshot` whenever anything changes (debounce bursts ~30 ms).
- `app.rs`: `State` + `impl telmo_kit::App` (key handling, events, no drawing).
- `ui.rs`: drawing only, pure function of the app state.
- `main.rs`: parse `telmo_kit::cli::args()`, create channels, `backend::spawn`,
  draw cached snapshot first (`telmo_kit::cache`), `telmo_kit::run(app, rx)`.

## Rules

- Clean, simple code. Small functions, obvious names, no clever abstractions,
  no speculative options. Match the style of `crates/kit`.
- Comments only where the reason isn't obvious from the code.
- The UI must never block. Anything slow (scans, connects, shelling out, D-Bus,
  IOBluetooth) happens in the backend.
- No `unwrap()`/`expect()` on anything that touches the system. Turn failures
  into a user-facing sentence: what went wrong and what to do.
- Every action shows a pending state on its own row and ends in either a new
  snapshot or a toast. Nothing fails silently.
- Visuals: follow `docs/mockups/*.txt` (exact text renders of every screen,
  90×22) and `docs/mockups/index.html`. Use `telmo_kit::widgets` and
  `telmo_kit::theme`; don't invent new colors or border styles.
- Display rules: list rows show only what you act on (name, state, one
  strength cue). Numbers go in a details dialog. The key bar shows at most 5
  keys plus `?` (help lists everything) and `esc`.
- Esc closes the top dialog; with no dialog open it quits. `q` also quits.
- Snapshot tests: one per mockup state, using `telmo_kit::test::render(90, 22, ..)`
  and `insta::assert_snapshot!`, driven by the mock backend's data.

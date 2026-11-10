# Contributing to pz-rs

pz-rs is a Rust reimplementation of the Project Zomboid (Build 42) engine.
This file covers what you need to set up, how to find work, and what a pull
request must satisfy. For the state of the project and the list of open work,
read [docs/STATUS.md](docs/STATUS.md) first.

## Hard rules

These two rules come from the [README](README.md) and are not negotiable:

1. **Never commit original game material.** No decompiled source, Lua
   scripts, assets, extracted data, fixtures or golden files generated from
   the game. Keep all of it under the git-ignored `local/` folder.
   `.gitignore` also blocks the common game file types (`*.lua`, `*.java`,
   `*.class`, `*.jar`, `*.png`, `*.pack`, `*.bank`, `*.fnt`, …); do not
   force-add around it.
2. **100% Rust at runtime.** No JVM, no Java bridge, no C Lua.

A consequence of rule 1: tests that need game data must read it from the
contributor's own install at run time and skip when it is absent. Parity
results are recorded as pass/fail notes, never as committed game data
([docs/WORKSTREAMS.md](docs/WORKSTREAMS.md#reference-material)).

## What you need

- A stable Rust toolchain (`rustup toolchain install stable`). CI uses
  stable on Linux x64, Windows x64, macOS Apple Silicon and macOS Intel.
- Your own installed copy of Project Zomboid Build 42. The engine reads its
  `media/` folder and `projectzomboid.jar` at run time.
- For porting work, a local decompile of `projectzomboid.jar` (`zombie.*`,
  `se.krka.kahlua.*`, `org.luaj.*`) to port from. It stays on your machine,
  under `local/`.
- On Linux, the system packages CI installs for audio, input, windowing and
  graphics:

  ```sh
  sudo apt-get install --no-install-recommends \
    pkg-config libasound2-dev libudev-dev \
    libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev \
    libx11-dev libxrandr-dev libxi-dev libxcursor-dev libxinerama-dev \
    libegl1-mesa-dev libgl1-mesa-dev libgl1-mesa-dri \
    libvulkan1 mesa-vulkan-drivers
  ```

## Build, test, run

These are the two commands CI runs; run both before opening a pull request:

```sh
cargo build --locked -p pz-client
cargo test --locked --workspace --no-fail-fast
```

The full workspace is large (49 crates). While iterating, test only the
crates you touched, for example `cargo test -p pz-lua-api`.

To run the client against your install:

```sh
export PZ_GAME_DIR=/path/to/ProjectZomboid      # the directory containing media/
cargo run -p pz-client -- --new-game test --headless-frames 120
```

The client writes to `~/Zomboid-rs` (or `--cache-dir DIR`) and never to the
original game's `~/Zomboid`. See the [README](README.md#running) for the other
launch options.

### What CI does and does not prove

CI has no game assets. Asset-dependent tests are ignored or skip when their
environment variables are unset, and GPU tests may skip when a runner has no
suitable graphics adapter. A green run shows the source builds and the
default tests pass on four platforms; it does not establish gameplay parity.
If your change affects behaviour that only shows with real assets, run it
against your install and say what you ran in the pull request.

## Finding something to work on

[docs/STATUS.md](docs/STATUS.md) groups the open work. Three sizes of task:

- **Small: bind a missing Lua API method.** Run a headless session as above,
  take the first Lua error that names a missing method, and bind it in
  `pz-lua-api` with tests.
- **Medium: wire an existing library into the engine.** Pick an unchecked
  item in [docs/ENGINE-WIRING.md](docs/ENGINE-WIRING.md). Each entry names
  the functions to call and where they belong in the original's order. Most
  of the work lands in `pz-engine`.
- **Large: an unstarted area**, such as multiplayer. Open an issue to discuss
  the approach before writing code.

If you plan to take a medium or large item, open an issue (or comment on an
existing one) saying so, to avoid duplicated work.

## How to port

The goal is 1:1 behaviour with the original, so:

- **Port from the original, do not redesign.** Follow the original's order of
  operations, including quirks: evaluation order, when a field is read, which
  branch returns early, how random numbers are drawn. Mods and the game's own
  Lua depend on these details.
- **Name the original counterpart.** Module and item doc comments say which
  Java class or native library they correspond to (for example
  `zombie.iso.IsoCell` or `libPZPathFind`). Describe behaviour in your own
  words; do not paste decompiled code into comments.
- **Read constants from the install, not from memory.** Values defined in the
  jar are read at run time through `pz-classfile`; data defined in `media/`
  is read from `media/`.
- **Leave gaps explicit.** If a branch depends on something not yet ported,
  return a clear error or leave a marked TODO rather than a silent
  approximation.
- **Keep crates decoupled.** Subsystem crates expose hooks and traits; the
  glue crates (`pz-world-states`, `pz-world-popman`, `pz-world-sound`,
  `pz-world-render`) and `pz-engine` connect them. Avoid making two subsystem
  crates depend on each other directly.

## Tests

- Add unit tests with every change. Build inputs in the test itself; do not
  commit fixtures derived from game files.
- Where behaviour can be compared with the original, do the comparison
  locally (several crates have `examples/` and oracle-directory environment
  variables for this) and report the result in the pull request.
- Do not skip, ignore or delete an existing test to make a change pass.

## Pull requests

- One topic per pull request. Keep unrelated refactors out.
- Commit messages: a short imperative subject line describing the behaviour
  change (for example "Stop virtual animal movement on an actual null junction
  list"), with a body when the reason is not obvious.
- In the description, state what changed, how you tested it, and whether you
  ran it against installed assets.
- Update the docs your change affects: tick or add the item in
  `docs/ENGINE-WIRING.md` when you wire something, and adjust
  `docs/STATUS.md` if a workstream's status changes.
- All four platform jobs must pass.

## License

Contributions are accepted under the repository's [MIT License](LICENSE).
Bundled third-party code keeps its own licenses; see the
[README](README.md#license). By contributing you confirm the work is your own
and contains no material copied from Project Zomboid.

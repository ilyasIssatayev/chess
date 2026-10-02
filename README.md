# Chess camera recorder

Planned macOS application using the built-in MacBook camera to observe a physical chess game, record moves and elapsed move times, store games locally, and provide a replay dashboard with PGN export for Chess.com.

The priority is precise move recording, with explicit review when the visual evidence is ambiguous.

See the [phased multi-agent development plan](docs/development-plan.md) for architecture, agent responsibilities, milestone gates, vision model candidates, timing semantics, database design, and validation targets.

The repository currently contains a Rust binary scaffold. Once a Rust toolchain is available:

```sh
cargo run
```

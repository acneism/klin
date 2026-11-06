# Klin

A desktop web browser for Windows built on the [Servo](https://servo.org) engine. All of
Klin's own code is written in Rust. Features and behavior follow Firefox: keyboard
shortcuts, data formats, settings.

> **Status: stage 0, prototype.** Klin opens a window with a Servo-rendered page, and
> Firefox keyboard shortcuts work through the core. There is no on-screen tab strip or
> address bar yet; they arrive with the egui interface. Not ready for everyday use.

The specification and architecture are in [docs/spec.md](docs/spec.md) (in Russian).

## Building

Requirements (details in [CONTRIBUTING.md](CONTRIBUTING.md)): Rust 1.88+ with the
`x86_64-pc-windows-msvc` host, Visual Studio 2022 (MSVC v143, ATL, Windows SDK) and Clang 19+.

```bash
cargo test
```

Runs the core and service tests without Servo; takes a few seconds.

```bash
cargo run -p klin-app -- https://servo.org
```

Full build with the engine; the first build takes a while.

## Layout

A single Cargo workspace of 15 crates. Only `crates/engine` depends on `servo`, so the
monthly breaking changes in Servo's API are fixed in one place.

| Crate | Purpose | State |
| --- | --- | --- |
| `app` | the `klin` binary, winit event loop | prototype |
| `shared` | IDs and shared types | done for 0.1 |
| `core` | window and tab model, commands, shortcuts, URIFixup, zoom | 0.1 foundation, covered by tests |
| `engine` | `ServoEngine`: the `Engine` trait implemented on servo 0.7.0 | prototype |
| `internal-pages` | about: pages served through the `klin:` scheme | about:newtab |
| `ui` | egui interface | next step |
| `profile`, `storage`, `sync`, `remote-settings`, `content-blocking`, `downloads`, `extensions`, `importer`, `platform-win` | services and Windows integration | stubs, scheduled per the version plan |

## License

[MPL 2.0](LICENSE), same as Firefox and Servo. The Firefox name, logos and Mozilla icons
are not used.

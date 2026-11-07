# Klin

A desktop web browser for Windows built on the [Servo](https://servo.org) engine. All of
Klin's own code is written in Rust. Features and behavior follow Firefox: keyboard
shortcuts, data formats, settings.

> **Status: stage 0, prototype.** Windows (Ctrl+N, private Ctrl+Shift+P) with a tab strip, navigation buttons and an
> address bar drawn with egui; pages are rendered by Servo. Firefox keyboard shortcuts work
> through the core (Ctrl+T, Ctrl+W, Ctrl+Shift+T, Ctrl+Tab, Ctrl+L, Alt+←/→, F5, Ctrl+±/0 and
> more). Page dialogs, permission prompts, HTTP auth, the context menu, `<select>` popups
> and the native file picker on Windows work. The profile lives in `%APPDATA%\Klin` in
> Firefox's layout (`-profile <path>` to override); per-site zoom is saved in
> `content-prefs.sqlite`. Stop (Esc) and file downloads work without changes to Servo,
> with recorded limitations (see the spec). History and bookmarks are stored in
> `places.sqlite` (ported from Mozilla's application-services): URL bar suggestions ranked by
> frecency, Ctrl+D with a star button, Ctrl+H history sidebar. The session is saved in
> Firefox's `sessionstore.jsonlz4` format and restored after a crash, or after a normal exit with
> `browser.startup.page = 3` in `prefs.js`/`user.js`. Find in page (Ctrl+F, F3) works through
> the selection API. `klin -migration` imports history, bookmarks and cookies from the default
> Firefox profile. Settings: the ≡ menu → «Настройки» (about:preferences) and about:config,
> stored in `prefs.js` like Firefox. The Library (Ctrl+Shift+H, Ctrl+Shift+O) shows history,
> downloads and all bookmark folders with search and deletion. Reader View (F9, Ctrl+Alt+R) uses
> Mozilla's Readability.js and Firefox's reader styles. Not ready for everyday use.

The specification and architecture are in [docs/spec.md](docs/spec.md) (in Russian).

## Building

Requirements (details in [CONTRIBUTING.md](CONTRIBUTING.md)): Rust 1.89+ with the
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
| `ui` | egui interface: tabs, navigation, address bar, link status, crash page | prototype |
| `profile` | profile folder, `profiles.ini`, `parent.lock` | done for 0.1 |
| `storage` | `content-prefs.sqlite` (per-site zoom); `places.sqlite` history and bookmarks, ported from application-services | history, bookmarks, suggestions |
| `downloads` | download service: background HTTP download with the engine's cookies, Firefox-style file names | done for 0.1 |
| `platform-win` | open a file, show it in Explorer | started |
| `importer` | reads history, bookmarks and cookies from a Firefox profile | done for 0.2 |
| `sync`, `remote-settings`, `content-blocking`, `extensions` | services and Windows integration | stubs, scheduled per the version plan |

## License

[MPL 2.0](LICENSE), same as Firefox and Servo. The Firefox name, logos and Mozilla icons
are not used.

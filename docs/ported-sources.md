# Перенесённый код и источники

Требование ТЗ: перенесённый код остаётся под MPL 2.0 и ссылается на исходник. Ссылки собраны здесь.

| Код Klin | Источник | Что изменено |
| --- | --- | --- |
| `crates/storage/src/places/`, `crates/storage/sql/places/` | mozilla/application-services, `components/places`, коммит `96b6f69902d143d6257c176c6e49e8c21e6d314b` | rusqlite 0.38; убраны синхронизация, uniffi/FFI, glean, импорт из iOS, `history_metadata` (оставлено удаление) |
| `crates/storage/src/support/{sql,guid,types,interrupt}` | там же, `components/support` | пути модулей; `error-support` заменён модулем `support/error.rs` (только лог) |
| `crates/storage/src/places/sync_meta.rs` | `places/src/bookmark_sync/{engine,mod}.rs`, `places/src/history_sync/engine.rs`, `sync15/src/engine/sync_engine.rs` | только константы и типы |
| `crates/storage/src/content_prefs.rs` | Firefox, `toolkit/components/contentprefs/ContentPrefService2.sys.mjs` (схема) | — |
| `crates/storage/src/sessionstore.rs` | Firefox, `browser/components/sessionstore` (формат и правила) | подмножество полей |
| `crates/core/src/urifixup.rs` | Firefox, `docshell/base/URIFixup.sys.mjs` | упрощено, без PSL |
| `crates/core/src/prefs.rs` (разбор `prefs.js`) | Firefox, `modules/libpref/parser` | без `sticky_pref`/`locked` |
| `crates/core/src/command.rs` | Firefox, `browser/base/content/browser-sets.inc` | имена команд и клавиши |
| `crates/downloads/src/names.rs` | Firefox, `toolkit/components/downloads/DownloadPaths.sys.mjs` | по образцу |

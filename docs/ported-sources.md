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
| `crates/engine/js/readability/` | mozilla/readability 0.6.0 (Apache 2.0, лицензия в `LICENSE.md` рядом) | убраны комментарии и `module.exports`, код не менялся |
| `crates/internal-pages/pages/reader.css` | Firefox, `toolkit/themes/shared/aboutReader.css` | без панели настроек, HCM и пользовательских цветов; вложенные правила развёрнуты; тема `auto` через `prefers-color-scheme` |
| `crates/internal-pages/src/reader.rs` | Firefox, `toolkit/components/reader/AboutReader.sys.mjs`, `ReaderMode.sys.mjs` | разметка статьи, скорость чтения по языкам, `_stripHost`; очистка HTML через ammonia вместо `nsIParserUtils` |
| `crates/core/src/reader.rs`, `crates/engine/js/readerable.js` | Firefox, `toolkit/components/reader/Readerable.js` | `shouldCheckUri` и список хостов; проверка HTML-документа через `HTMLHtmlElement` |
| `crates/content-blocking/src/` | Firefox, `toolkit/components/url-classifier` (правила функций Tracking/Social/Cryptomining/Fingerprinting/Email) и `browser/components/protections` (категории Standard/Strict); формат списков — mozilla-services/shavar-prod-lists | сопоставление по суффиксам хоста (до 5 частей) вместо хешей Safe Browsing; белый список сущностей из `disconnect-entitylist.json` |
| `crates/internal-pages/resources/pdfjs/` | mozilla/pdf.js, пакет pdfjs-dist 6.5.438 (Apache 2.0; лицензии шрифтов и wasm-декодеров лежат рядом) | без изменений; `pdf_viewer.mjs` минифицирован, из `pdf_viewer.css` убраны комментарии |
| `crates/internal-pages/resources/pdfviewer/viewer.mjs` | Firefox, `pdf.js/web/app.js` (`_initializeMetadata`: выбор заголовка документа) | только выбор заголовка; остальная панель своя |
| `crates/storage/src/logins/` | mozilla/application-services, `components/logins`, коммит `96b6f69902d143d6257c176c6e49e8c21e6d314b` | убраны синхронизация, uniffi, NSS и `LoginsApiError` (методы `LoginStore` возвращают внутреннюю ошибку); тесты, завязанные на зеркало синхронизации, убраны |
| `crates/storage/src/support/crypto.rs` | там же, `components/support/db-crypto` и `components/support/jwcrypto` (JWE `dir` + A256GCM, ключ в JWK) | своя реализация на RustCrypto (`aes-gcm`) вместо NSS; формат совместим, проверено тестовым вектором node-jose из application-services |
| `crates/importer/src/nss.rs` | формат NSS: `key4.db` (таблицы `metaData`, `nssPrivate`, PBES2 и PBE-SHA1-3DES) и шифротекст SDR из `logins.json` (NSS, `lib/pk11wrap/pk11sdr.c`, `lib/softoken/lowpbe.c`) | своя реализация на RustCrypto без NSS; тестовые базы в `crates/importer/tests/data` созданы системной NSS (`certutil`, `PK11SDR_Encrypt`) |

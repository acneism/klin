# Klin

Десктопный браузер для Windows на движке [Servo](https://servo.org). Весь собственный код
написан на Rust. Функции и поведение берём у Firefox: горячие клавиши, форматы данных,
тексты настроек.

> **Статус: этап 0, прототип.** Открывает окно со страницей Servo, горячие клавиши Firefox
> работают через ядро. Вкладок на экране и адресной строки ещё нет: они появятся вместе с
> интерфейсом egui. Ежедневно пользоваться браузером пока нельзя.

Техническое задание и архитектура лежат в [docs/spec.md](docs/spec.md).

## Сборка

Требования (подробности в [CONTRIBUTING.md](CONTRIBUTING.md)): Rust 1.88+ с хостом
`x86_64-pc-windows-msvc`, Visual Studio 2022 (MSVC v143, ATL, Windows SDK) и Clang 19+.

```bash
cargo test                   # ядро и сервисы без Servo, несколько секунд
cargo run -p klin-app -- https://servo.org   # полная сборка с движком, первый раз долго
```

## Устройство

Один Cargo workspace из 15 крейтов. От `servo` зависит только `crates/engine`, так что
ежемесячные ломающие изменения API Servo правятся в одном месте.

| Крейт | Назначение | Состояние |
| --- | --- | --- |
| `app` | бинарник `klin`, цикл winit | прототип |
| `shared` | идентификаторы и общие типы | готово для 0.1 |
| `core` | модель окон и вкладок, команды, горячие клавиши, URIFixup, масштаб | основа 0.1, покрыта тестами |
| `engine` | `ServoEngine`: реализация трейта `Engine` поверх servo 0.7.0 | прототип |
| `internal-pages` | about:-страницы через схему `klin:` | about:newtab |
| `ui` | интерфейс на egui | следующий шаг |
| `profile`, `storage`, `sync`, `remote-settings`, `content-blocking`, `downloads`, `extensions`, `importer`, `platform-win` | сервисы и интеграция с Windows | заготовки, по плану версий |

## Лицензия

[MPL 2.0](LICENSE), как у Firefox и Servo. Название «Firefox», логотипы и иконки Mozilla
не используются.

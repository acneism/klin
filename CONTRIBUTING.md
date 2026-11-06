# Как участвовать

## Окружение для Windows

По [Servo Book для Windows](https://book.servo.org/building/windows.html):

- rustup с хостом `x86_64-pc-windows-msvc` (32-битный хост по умолчанию — известная причина ошибок), Rust 1.88+;
- Visual Studio 2022: MSVC v143 build tools (Latest), C++ ATL для v143, Windows SDK 10.0.19041+;
- Clang 19+ (обязателен с Servo 0.6);
- если сборка mozjs падает с ошибкой `GetShortPathName`, включите короткие имена 8.3.

> TODO (этап 0, первая неделя): записать сюда, что ещё нужно при сборке через крейт с
> crates.io (Python, GStreamer SDK для `media-gstreamer`).

Проверено на Linux (Ubuntu 24.04, Clang 18): `cargo check --workspace` и `cargo clippy`
с servo 0.7.0 проходят, `cargo build -p klin-app` собирается за ~5 минут на 4 ядрах,
а бинарник под Xvfb показывает страницу file:// и about:newtab.

## Правила

- **Ничего не придумываем.** У функции есть эталон в Firefox. Отступление допускается
  только с комментарием `Отступление от Firefox: …` и причиной.
- От `servo` зависит только `crates/engine`. `ui` вызывает движок только через команды `core`.
- `unsafe` разрешён только в `crates/platform-win`, и у каждого блока есть комментарий `// SAFETY:`.
- Перенесённый из Firefox код остаётся под MPL 2.0, а в файле есть ссылка на исходник.
- Перед коммитом выполняйте `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` и `cargo test`.

## Обновление Servo

Версия зафиксирована точно (`servo = "=0.N.0"` в корневом `Cargo.toml`). Обновление делается
отдельной веткой раз в месяц: читаем пост в блоге Servo, правим только `crates/engine`,
затем прогоняем тесты и дымовой проход по сайтам. Патч в Servo не живёт у нас дольше двух выпусков.

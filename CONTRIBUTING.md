# Как участвовать

## Окружение для Windows

По [Servo Book для Windows](https://book.servo.org/building/windows.html):

- rustup с хостом `x86_64-pc-windows-msvc` (32-битный хост по умолчанию — известная причина ошибок), Rust 1.89+;
- Visual Studio 2022: MSVC v143 build tools (Latest), C++ ATL для v143, Windows SDK 10.0.19041+;
- Clang 19+ (обязателен с Servo 0.6);
- если сборка mozjs падает с ошибкой `GetShortPathName`, включите короткие имена 8.3;
- GStreamer 1.24 для MSVC x86_64: оба установщика с
  [gstreamer.freedesktop.org](https://gstreamer.freedesktop.org/download/) — runtime и devel
  (`gstreamer-1.0-msvc-x86_64-1.24.13.msi` и `gstreamer-1.0-devel-msvc-x86_64-1.24.13.msi`,
  «Complete»), и `pkg-config` (например, `choco install pkgconfiglite`). Переменные:
  `GSTREAMER_1_0_ROOT_MSVC_X86_64=C:\gstreamer\1.0\msvc_x86_64\`,
  `PKG_CONFIG_PATH=%GSTREAMER_1_0_ROOT_MSVC_X86_64%lib\pkgconfig`, а в `PATH` — его `bin`.

Servo на Windows загружает плагины GStreamer из папки рядом с `klin.exe` и без них не
запускается, поэтому после сборки выполните:

```powershell
./tools/copy-gstreamer.ps1 -Target target/debug
```

На Linux нужны пакеты `libgstreamer1.0-dev`, `libgstreamer-plugins-base1.0-dev`,
`libgstreamer-plugins-bad1.0-dev` и плагины `gstreamer1.0-plugins-{base,good,bad}`,
`gstreamer1.0-libav`.

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

# Klin: браузер на Servo — ТЗ и архитектура

Nov 6, 2026 · @Dias

## Цели и рамки

Делаем десктопный браузер для Windows на движке Servo. Функции и поведение копируем с Firefox, весь собственный код пишем на Rust.

**Принципы**

1. **Ничего не придумываем.** У каждой функции есть эталон в Firefox: поведение, горячие клавиши, формат данных, тексты настроек. Отклонение от Firefox допускается только с записанной причиной (например, «в Servo нет нужного API»).
2. **Порядок заимствования:**
   1. готовый Rust-крейт Mozilla подключаем как зависимость;
   2. если крейта нет — переносим код Firefox на Rust (MPL 2.0);
   3. если код намертво связан с Gecko — воспроизводим поведение по образцу.
3. **Чистый Rust для нашего кода.** C/C++ остаётся только внутри зависимостей движка (SpiderMonkey, GStreamer и др.), в системных API Windows и, пока нет замены, в NSS для Firefox Sync (см. открытые вопросы).
4. **Servo не форкаем.** Чего не хватает в движке — дописываем патчами в upstream Servo, сами сидим на опубликованных версиях крейта.
5. **Нативный интерфейс:** winit + egui, как в servoshell; страницы рисует Servo, всё остальное — наш код.

**Входит в проект**

- Windows 10/11 x64 — единственная поддерживаемая платформа первых версий.
- Окна, вкладки, адресная строка, навигация, профиль пользователя, закладки, история, загрузки, пароли, настройки, импорт из Firefox, Firefox Sync, режим чтения, PDF, подмножество WebExtensions.

**Не входит (на первых этапах)**

- Сборки для macOS, Linux, Android (код держим кроссплатформенным, но не тестируем).
- DRM (EME, Widevine) и всё, что от него зависит.
- Собственный форк Servo и собственный JS-движок.
- Полная совместимость с каталогом addons.mozilla.org.
- Свой сервер синхронизации: используем инфраструктуру Firefox Sync.

## Функциональные требования

Функции разбиты на четыре версии. Каждая строка принимается, когда поведение совпадает с эталоном из Firefox: те же горячие клавиши, те же состояния, те же данные на диске, где это возможно.

| Версия | Функция | Эталон в Firefox | Что нужно от Servo |
| --- | --- | --- | --- |
| 0.1 | Окна и вкладки: открыть, закрыть, перетащить, вернуть закрытую | Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+T, средняя кнопка мыши | несколько WebView, несколько окон — есть |
| 0.1 | Адресная строка: адрес или поисковый запрос | Ctrl+L, Alt+D, F6, разбор ввода как в URIFixup | — |
| 0.1 | Навигация: назад, вперёд, обновить, стоп, домой | Alt+←/→, F5, Ctrl+F5, Esc, Alt+Home | история сессии WebView есть; метода остановки загрузки в API нет — Stop через `window.stop()` (`evaluate_javascript`); отступление: переход, ещё не получивший ответ, не отменяется |
| 0.1 | Заголовок, значок сайта, индикатор загрузки, статус ссылки | то же поведение, что во вкладках Firefox | события делегата WebView — есть |
| 0.1 | Контекстное меню, alert/confirm/prompt, выбор файла | пункты меню и модальные диалоги вкладки | запросы к встраивателю — есть |
| 0.1 | Масштаб страницы | Ctrl+±, Ctrl+0, масштаб запоминается по сайту | `WebView::set_page_zoom` — есть |
| 0.1 | Профиль в %APPDATA%, базовые загрузки | структура папки профиля Firefox | дисковый HTTP-кэш есть за настройкой `network_http_disk_cache`; событий загрузки файлов в API нет — загрузка распознаётся по расширению в URL через `WebResourceLoad::intercept`, файл скачивает служба `downloads` с cookies движка; отступление: `Content-Disposition` без расширения в URL и POST-загрузки не распознаются |
| 0.2 | История и библиотека | Ctrl+H (боковая панель), Ctrl+Shift+H (Библиотека), база places | — |
| 0.2 | Закладки: добавить, панель, менеджер | Ctrl+D, Ctrl+Shift+B, Ctrl+Shift+O | — |
| 0.2 | Подсказки адресной строки по истории и закладкам | умная строка (ранжирование по frecency) | — |
| 0.2 | Менеджер загрузок | Ctrl+J, панель загрузок | панель загрузок сделана в 0.1 (Ctrl+J открывает её, а не «Библиотеку»); в 0.2 — история загрузок в places |
| 0.2 | Восстановление сессии после закрытия и сбоя | sessionstore | восстановить вкладке список переходов «Назад/Вперёд» в API нет — восстанавливается текущая страница, список сохраняется в файле (отступление) |
| 0.2 | Приватное окно | Ctrl+Shift+P, отдельные cookies и кэш | отдельный контекст хранилища — проверить |
| 0.2 | Поиск на странице | Ctrl+F, F3, подсветка всех совпадений | в API WebView поиска нет — сделано через `evaluate_javascript` и Selection; отступление: «Подсветить все» недоступна (в Servo нет CSS Custom Highlight API, а `Range.getClientRects` неверен) |
| 0.2 | Настройки | разделы about:preferences и about:config | проброс настроек в `Preferences` Servo |
| 0.2 | Импорт из Firefox: закладки, история, cookies | мастер импорта данных Firefox | запись cookies в хранилище движка — проверить |
| 0.3 | Пароли: сохранение, автозаполнение, менеджер | about:logins, импорт паролей из профиля Firefox | отправка форм и заполнение полей — через `UserContentManager` и `evaluate_javascript` или патч |
| 0.3 | Аккаунт Mozilla и Firefox Sync: закладки, история, пароли, открытые вкладки | меню «Войти в аккаунт», «Вкладки с других устройств» | страница входа в аккаунт должна работать в Servo |
| 0.3 | Режим чтения | Reader View: Ctrl+Alt+R, F9 | есть: `evaluate_javascript`, `UserContentManager` |
| 0.3 | Встроенный просмотр PDF | PDF.js | достаточная совместимость движка с PDF.js |
| 0.3 | Защита от отслеживания по спискам | стандартный режим Enhanced Tracking Protection | есть: `WebResourceLoad::intercept` (только HTTP/HTTPS) |
| 0.4 | WebExtensions, часть 1: manifest, content scripts, storage, tabs, runtime, кнопка на панели | реализация WebExtensions и схемы API из Firefox | `UserContentManager` есть, изолированных миров нет — патч в Servo |
| 0.4 | WebExtensions, часть 2: блокирующий webRequest и declarativeNetRequest (цель — uBlock Origin) | то же | `WebResourceLoad::intercept` — проверить, хватит ли его для webRequest |
| 0.4 | Локальный перевод страниц | Firefox Translations | доступ к тексту DOM и его замена через `evaluate_javascript` |

«Проверить» означает: до начала работы над функцией свериться с актуальной документацией крейта servo и при необходимости заложить патч в движок.

## Нефункциональные требования

Интерфейс никогда не ждёт движок, данные пользователя не теряются при сбое, а безопасность не декларируется сильнее, чем есть на самом деле. Числа ниже — целевые; уточняем их после замеров на прототипе.

| Область | Требование |
| --- | --- |
| Отзывчивость | Поток интерфейса не делает дисковых и сетевых операций; кадр интерфейса не дольше 16 мс (60 кадров/с) даже при зависшей странице |
| Запуск | Холодный старт до первого кадра окна — меньше 2 с на SSD; восстановленные вкладки грузятся лениво, как в Firefox |
| Память | Фоновые вкладки можно выгрузить (как tab unloading в Firefox); страница about:memory для диагностики |
| Надёжность данных | Все базы профиля — SQLite в режиме WAL; сессия сохраняется атомарно (запись во временный файл + переименование), не реже раза в 15 с, как `browser.sessionstore.interval` в Firefox |
| Устойчивость к сбоям | Паника в коде страницы не роняет интерфейс: Servo сообщает о ней через `WebViewDelegate::notify_crashed`, вкладка показывает страницу сбоя с кнопкой «Перезагрузить»; отчёт о сбое пишется локально и отправляется только с согласия |
| Изоляция | Многопроцессный режим Servo на Windows работает, но без песочницы: песочница Servo реализована только для macOS и Linux. Процессы используем для изоляции сбоев; песочница для Windows — открытая задача, и до её решения «О программе» честно сообщает, что её нет |
| Секреты | Пароли и токены аккаунта шифруются; ключ хранится через Windows DPAPI (своя реализация `KeyManager` из компонента logins). Это записанное отступление: в Firefox ключ лежит в NSS key4.db под основным паролем, а мы не тянем NSS ради одного ключа. Для показа пароля — повторная авторизация в ОС, как в Firefox |
| Обновления | Новая версия движка — не реже раза в месяц; исправления безопасности Servo — в течение 7 дней после выхода; обновление без потери профиля |
| Приватность | Никакой телеметрии по умолчанию; все сетевые запросы самого браузера (обновления, списки, Sync) перечислены в документации |
| Windows | Windows 10 22H2 и Windows 11, x64. Microsoft прекратила поддержку Windows 10 14 октября 2025 года, но Firefox продолжает её поддерживать — следуем Firefox. Пер-мониторный DPI, тёмная тема системы, раскладки и IME, регистрация браузером по умолчанию, установка без прав администратора |
| Доступность | Интерфейс целиком управляется с клавиатуры. Дерево доступности страниц Servo уже отдаёт через AccessKit (пока экспериментально, настройка `accessibility_enabled`); встраиваем его в дерево окна |
| Качество кода | Весь наш код — стабильный Rust; `unsafe` только в модуле интеграции с Windows, с комментарием в каждом блоке; clippy без предупреждений в CI |

## Архитектура: обзор

Браузер — четыре слоя над Servo; на первом этапе всё работает в одном процессе Windows. Интерфейс, ядро и адаптер движка работают в главном потоке вместе с циклом событий Servo. Всё, что ходит на диск или в сеть от имени браузера, вынесено в фоновые сервисы.

&#91;embedded content: слои браузера · 4 слоя над Servo\]

События идут обратно снизу вверх: Servo сообщает о загрузке, смене адреса и новом кадре через `WebViewDelegate`, адаптер превращает их в события ядра, ядро меняет модель вкладок, интерфейс перерисовывается.

- **Главный поток.** Цикл winit → кадр egui → `Servo::spin_event_loop`; движок будит цикл через `EventLoopWaker`. API servo рассчитано на один поток (`WebView` — это `Rc<RefCell<…>>`, делегаты передаются как `Rc`), поэтому весь код, касающийся `WebView`, живёт здесь.
- **Фоновые потоки.** Сервисы общаются с ядром только сообщениями; готовый ответ кладётся в очередь главного потока и будит цикл событий.
- **Процессы.** Многопроцессный режим Servo на Windows уже есть (Cargo-фича `multiprocess` включена по умолчанию), но без песочницы. На прототипе проверяем его стабильность и включаем для изоляции сбоев: сайты уходят в отдельные процессы (`run_content_process`), а интерфейс и сервисы остаются в главном, как в Firefox.
- **Отрисовка.** Страницу рисует Servo в свой `RenderingContext`, а egui собирает её внутри окна вместе с интерфейсом, как в servoshell. Точную схему контекстов (оконный или внеэкранный) выбираем на прототипе. Фокус ввода между egui и страницей передаём сами через `WebView::set_focused`.
- **Внутренние страницы.** Интерфейс окна нативный, а about:newtab, about:preferences, страницы ошибок и чтения — HTML, как в Firefox. Схему about: Servo зарегистрировать не даёт: она в списке запрещённых вместе с http, https и chrome. Поэтому страницы отдаём через свою схему (так servoshell отдаёт servo:preferences), а адресная строка показывает и принимает привычные about:-адреса. Регистрация — через `ProtocolRegistry` Servo; страницам настроек нужны привилегии, их даёт `privileged_paths` обработчика.

## Модули

Проект — один Cargo workspace из 15 крейтов. Главное правило: от крейта `servo` зависит только `engine`. Когда API Servo меняется (а в ежемесячных выпусках это ожидаемо), правится один крейт, а не весь браузер. Именно на этом потерял темп Verso.

```text
browser/
  Cargo.toml              workspace, общие версии зависимостей
  crates/
    app/                  бинарник: запуск, цикл winit, сборка всех частей
    shared/               общие типы: TabId, WindowId, события, ошибки
    core/                 модель вкладок и окон, сессия, команды, prefs
    ui/                   egui: вкладки, адресная строка, меню, панели, диалоги
    engine/               единственный консумент servo
    internal-pages/       about:-страницы (HTML/CSS/JS вшиваются в бинарник)
    profile/              папка профиля, блокировка, миграции, ключи
    storage/              places, logins, sessionstore
    sync/                 аккаунт Mozilla + Firefox Sync
    remote-settings/      поисковики, списки защиты от отслеживания
    content-blocking/     правила блокировки и их сопоставление с запросами
    downloads/            очередь загрузок, история загрузок
    extensions/           WebExtensions: manifest, хост, API browser.*
    importer/             импорт профиля Firefox
    platform-win/         DPAPI, браузер по умолчанию, ярлыки, уведомления, обновление
```

| Крейт | Отвечает за | Может зависеть от |
| --- | --- | --- |
| app | запуск, аргументы командной строки, один экземпляр на профиль, цикл событий | все крейты |
| core | состояние браузера и правила его изменения; не знает ни про egui, ни про servo | shared |
| ui | отрисовка состояния core и превращение действий в команды | core, shared, egui |
| engine | реализация трейта `Engine` поверх servo | core, shared, servo, internal-pages, content-blocking |
| storage, sync, remote-settings, downloads | фоновые сервисы; наружу — только асинхронный интерфейс сообщений | shared, profile, компоненты Mozilla |
| extensions | жизненный цикл расширений и API `browser.*` | core, shared, storage |
| platform-win | всё, что требует Win32; единственное место с `unsafe` | crate `windows` |

**Ключевые интерфейсы ядра.** Ядро говорит с движком через трейт и два перечисления. Имена команд берём из Firefox (`Browser:Back`, `cmd_newNavigatorTab`, `History:UndoCloseTab`), чтобы горячие клавиши и пункты меню переносились один в один.

```rust
// core: всё, что ядро знает о движке
pub trait Engine {
    fn open_tab(&mut self, window: WindowId, url: &Url, private: bool) -> TabId;
    fn close_tab(&mut self, tab: TabId);
    fn load(&mut self, tab: TabId, url: &Url);
    fn go_back(&mut self, tab: TabId);
    fn go_forward(&mut self, tab: TabId);
    fn reload(&mut self, tab: TabId, bypass_cache: bool);
    fn stop(&mut self, tab: TabId);
    fn set_zoom(&mut self, tab: TabId, factor: f32);
    fn send_input(&mut self, tab: TabId, input: InputEvent);
    fn answer(&mut self, request: RequestId, reply: Reply); // диалоги, разрешения, файлы
}

// engine -> core: что произошло в странице
pub enum EngineEvent {
    LoadStatus { tab: TabId, status: LoadStatus },
    UrlChanged { tab: TabId, url: Url },
    TitleChanged { tab: TabId, title: String },
    FaviconChanged { tab: TabId, icon: Image },
    HistoryChanged { tab: TabId, can_back: bool, can_forward: bool },
    OpenTabRequested { opener: TabId, url: Url, disposition: Disposition },
    Request { tab: TabId, id: RequestId, kind: RequestKind }, // alert, confirm, файл, разрешение, меню
    DownloadStarted { tab: TabId, download: DownloadInfo },
    NewFrame { tab: TabId },
    Crashed { tab: TabId, reason: String },
}

// ui -> core: команды с именами из browser-sets Firefox
pub enum Command { NewTab, CloseTab, UndoCloseTab, Back, Forward, Reload, Stop,
                   FocusUrlBar, Find, ZoomIn, ZoomOut, ZoomReset, Bookmark, /* ... */ }
```

**Правила зависимостей**

- `ui` никогда не вызывает `engine` напрямую — только через команды `core`.
- Сервисы не знают про поток интерфейса: получают запрос, отвечают сообщением.
- В `core` есть тестовая реализация `Engine` без Servo, чтобы логика вкладок, сессий и команд тестировалась за секунды и без сборки движка.

## Карта заимствований из Firefox

Хранение данных пользователя и синхронизация подключаются готовыми Rust-компонентами Mozilla из [application-services](https://github.com/mozilla/application-services); остальное переносим на Rust или воспроизводим по образцу. Основные потребители этих компонентов — Firefox для Android и iOS, часть уже вендорится и в десктопный Firefox. На crates.io они публикуются нерегулярно, поэтому подключаем их git-зависимостью с зафиксированным коммитом.

| Что берём из Firefox | Куда | Как | Лицензия |
| --- | --- | --- | --- |
| places: история, закладки, frecency, поиск для адресной строки | storage | перенос: крейт требует rusqlite 0.37, Servo 0.7.0 — 0.38, а две встроенные SQLite в одной программе Cargo не допускает; перенесено без синхронизации, uniffi и glean (ноябрь 2026) | MPL 2.0 |
| logins: хранение и шифрование паролей | storage | готовый крейт; шифрование — через трейт `EncryptorDecryptor` (можно своё, без NSS), ключ — через свой `KeyManager` ([документация](https://mozilla.github.io/application-services/book/rust-docs/logins/encryption/index.html)) | MPL 2.0 |
| fxa-client, sync15, sync\_manager, tabs: аккаунт и синхронизация | sync | готовые крейты; криптография Sync идёт через NSS (rc\_crypto) | MPL 2.0 |
| remote-settings: клиент рассылки настроек | remote-settings | готовый крейт | MPL 2.0 |
| webext-storage: `storage.sync` для расширений | extensions | готовый крейт | MPL 2.0 |
| Локализация: строки интерфейса Firefox в формате Fluent (.ftl), в том числе русские | ui | данные + крейт `fluent-bundle` (Project Fluent) | строки — MPL 2.0, крейт — Apache 2.0 / MIT |
| Команды и горячие клавиши (browser-sets, раскладка меню) | core, ui | данные, перенесённые в таблицу Rust | MPL 2.0 |
| Настройки: имена и значения по умолчанию, формат prefs.js | core | порт формата + данные | MPL 2.0 |
| URIFixup: разбор ввода в адресной строке | core | порт на Rust | MPL 2.0 |
| Поисковики: компонент search (`SearchEngineSelector`) поверх коллекции search-config-v2 из Remote Settings | core | готовый крейт; README помечает компонент как ещё строящийся ([README](https://raw.githubusercontent.com/mozilla-firefox/firefox/main/third_party/application-services/components/search/README.md)) | MPL 2.0; условия использования сервера — проверить |
| sessionstore: формат и правила восстановления | storage | порт на Rust | MPL 2.0 |
| Менеджер загрузок (состояния, имена файлов, проверки) | downloads | по образцу | — |
| Импорт профиля: places.sqlite, cookies.sqlite, logins.json + key4.db | importer | порт форматов; расшифровка key4.db на RustCrypto | MPL 2.0 |
| Списки Enhanced Tracking Protection (от Disconnect) и логика классификации URL | content-blocking | данные + порт логики | списки — CC BY-NC-SA 4.0, для коммерческого использования нужна лицензия Disconnect ([репозиторий](https://github.com/disconnectme/disconnect-tracking-protection/)); код — MPL 2.0 |
| Схемы API WebExtensions (JSON) | extensions | данные: генерация Rust-типов и проверок аргументов | MPL 2.0 и BSD-3 (часть схем унаследована из Chromium) |
| Реализация WebExtensions (ext-\*.js, C++) | extensions | по образцу | — |
| Режим чтения: Readability.js + стили aboutReader | internal-pages | JS как есть, внутри страницы | Apache 2.0 / MPL 2.0 |
| Просмотр PDF: PDF.js | internal-pages | JS как есть, внутри страницы | Apache 2.0 |
| Перевод страниц (Firefox Translations, Bergamot) | extensions или отдельный крейт | решение отложено до 0.4 | MPL 2.0 |

**Способы.** «Готовый крейт» — зависимость без изменений; правки отправляем в Mozilla. «Порт» — переписываем на Rust, каждый такой файл остаётся под MPL 2.0 и содержит ссылку на исходник Firefox. «По образцу» — пишем сами, сверяясь с поведением и тестами Firefox.

**Исключение из «чистого Rust» для внутренних страниц.** Внутренние страницы (режим чтения, PDF, about:-страницы) в Firefox — это HTML и JS, и мы берём их такими же. Переписать PDF.js на Rust — отдельный большой проект, и это нарушило бы принцип «ничего не придумываем».

**Товарные знаки.** Название «Firefox», логотип и иконки не используем: на них MPL не распространяется. Из строк локализации брендовые названия заменяем на своё имя браузера.

## Интеграция с Servo

Сидим на ежемесячных выпусках крейта `servo` с crates.io и обновляемся каждый месяц отдельной задачей. LTS на этапе разработки не подходит: в ней только исправления безопасности, а нам нужны новые возможности движка и собственные патчи.

**Версии.** Первый выпуск крейта — 0.1.0 от 13 апреля 2026 года, дальше очередная версия выходит каждый месяц и может ломать API ([блог Servo](https://servo.org/blog/2026/04/13/servo-0.1.0-release/)). LTS-ветка появляется раз в 6 месяцев, поддерживается примерно 9 месяцев и получает только исправления безопасности; патч-выпуски LTS могут требовать git-зависимостей ([Servo Book, LTS](https://book.servo.org/embedding/lts-release.html)). На 6 ноября 2026 года последняя версия — 0.7.0 от 5 октября 2026 года, минимальная версия Rust — 1.88. LTS-линия — 0.1.x, последний выпуск 0.1.4 от 2 октября 2026 года; по графику её поддержка закончится примерно в январе 2027 года. Следующая LTS по графику ожидалась около октября 2026 года, но на дату проверки не объявлена. На LTS переходим, когда у браузера появится стабильный канал.

**Что из API Servo закрывает наши потребности.** Названия сверены с экспортами крейта servo 0.5.0, изменениями 0.6–0.7 из блога Servo и документацией ветки main на doc.servo.org ([исходник lib.rs](https://docs.rs/crate/servo/0.5.0/source/lib.rs)); точные возможности каждого типа проверяем на прототипе.

| Нам нужно | API Servo | Наш крейт |
| --- | --- | --- |
| Запустить движок, будить цикл событий | `ServoBuilder`, `ServoDelegate`, `EventLoopWaker`, `Servo::spin_event_loop` | engine |
| Вкладка и навигация | `WebViewBuilder`, `WebView` (`load`, `reload`, `go_back`/`go_forward`, `can_go_back`/`can_go_forward`, `clear_session_history`); метода остановки загрузки нет | engine |
| Фокус ввода между интерфейсом и страницей | `WebView::set_focused` (с 0.7.0 встраиватель вызывает его сам) | engine, ui |
| Отрисовка | `RenderingContext`: `WindowRenderingContext`, `OffscreenRenderingContext`, `SoftwareRenderingContext` | engine, ui |
| События страницы | `WebViewDelegate`: `notify_new_frame_ready`, `notify_url_changed`, `notify_load_status_changed`, `notify_history_changed`, `notify_crashed` | engine |
| Диалоги и запросы | `AlertDialog`, `ConfirmDialog`, `PromptDialog`, `FilePicker`, `ColorPicker`, `SelectElement`, `ContextMenu`, `PermissionRequest`, `AuthenticationRequest` | engine → ui |
| Новые вкладки и окна от сайта | `CreateNewWebViewRequest` (`request_create_new`), `NavigationRequest` | engine |
| Блокировка запросов (защита от отслеживания, webRequest) | `WebResourceLoad::intercept`: подмена или отмена ответа, только HTTP/HTTPS | engine, content-blocking |
| Скрипты в странице (чтение, автозаполнение, content scripts) | `UserContentManager` (скрипты и стили, без изолированных миров), `WebView::evaluate_javascript` | engine, extensions |
| Внутренние страницы | `ProtocolRegistry`, `ProtocolHandler`; схемы http, https, chrome и about зарегистрировать нельзя ([исходник](https://docs.rs/servo-net/0.1.0/src/net/protocols/mod.rs.html)) | engine, internal-pages |
| Cookies и данные сайтов, кэш | `SiteDataManager`, `NetworkManager`; дисковый HTTP-кэш — настройка `network_http_disk_cache` | engine |
| Снимки страниц (превью вкладок, тесты) | `WebView::take_screenshot` | engine |
| Буфер обмена, IME | `ClipboardDelegate`, `InputMethodControl` | engine, platform-win |
| Настройки движка | `Preferences`, `Opts` | core → engine |
| Доступность | реэкспорт `accesskit`, `WebView::set_accessibility_active`, `notify_accessibility_tree_update` | ui |

В API нет поиска на странице, остановки загрузки, событий загрузки файлов и изолированных миров для content scripts (сверено по списку методов WebView и WebViewDelegate). Остановка и загрузки в 0.1 сделаны без патчей (см. таблицу функций). Для остального до начала работы проверяем, не появилось ли нужное в свежем выпуске; патчи — только написанные человеком (см. «Правило Servo об ИИ»).

**Экспериментальные возможности.** Часть веб-платформы в Servo реализована частично и выключена по умолчанию: IntersectionObserver, IndexedDB, WebGPU, Notification; ServiceWorker тоже выключен, а MSE в списке нет вовсе ([Servo Book](https://book.servo.org/design-documentation/experimental-features.html)). servoshell включает такие возможности экспериментальным режимом. На прототипе решаем, какие из них включить по умолчанию: совместимость с сайтами вырастет, но стабильность может упасть.

**Порядок ежемесячного обновления движка**

1. Версия зафиксирована точно: `servo = "=0.N.0"` в корневом Cargo.toml.
2. После выхода версии — отдельная ветка: читаем ежемесячный пост Servo, чиним только крейт `engine`.
3. Прогон: тесты адаптера + дымовой проход по списку сайтов со скриншотами (см. «Сборка и тестирование»).
4. Исправления безопасности Servo не ждут планового обновления.

**Правило Servo об ИИ.** Политика проекта Servo ([AI Usage Policy](https://github.com/servo/book/blob/main/src/policy/ai-usage.md)) запрещает вклад, созданный с помощью генеративных ИИ-инструментов: код, описания pull request, issues и комментарии. Поэтому патчи, написанные ИИ-ассистентом, в Servo не отправляются. Решение (ноябрь 2026): то, чего нет в публичном API, обходим без патчей и записываем как отступление от Firefox; патчи в Servo пишет и отправляет человек, без ИИ.

**Патчи в Servo.** Чего не хватает в движке, делаем в нашей ветке репозитория Servo и сразу отправляем pull request. Пока он не принят, подключаем ветку через `[patch.crates-io]`. Правило: ни один патч не живёт у нас дольше двух выпусков Servo — иначе мы фактически держим форк.

**Медиа.** Аудио и видео на десктопе требуют фичи `media-gstreamer` ([Servo Book](https://github.com/servo/book/pull/223/changes)); единственный рабочий медиабэкенд servo-media — GStreamer ([servo-media](https://github.com/eerii/media)). Значит, в установщик придётся включить среду выполнения GStreamer для Windows.

Фича `media-gstreamer` не входит в набор по умолчанию (в 0.7.0 по умолчанию включены `bundled`, `clipboard`, `js_jit`, `multiprocess`), её включаем явно. На Windows плагины GStreamer Servo ищет в папке рядом с исполняемым файлом ([servo.rs](https://doc.servo.org/src/servo/servo.rs.html)), это учитываем в установщике.

## Сборка, тестирование и поставка на Windows

Собираем обычным `cargo build` под MSVC, тестируем логику без движка, а движок — дымовыми прогонами по реальным сайтам. Распространяем подписанным установщиком с автообновлением по модели Firefox.

**Окружение разработчика** (по [Servo Book для Windows](https://book.servo.org/building/windows.html) и [README Servo](https://github.com/servo/servo))

- rustup с хостом `x86_64-pc-windows-msvc` (32-битный хост по умолчанию — известная причина ошибок); Rust не ниже 1.88 — минимальная версия для servo 0.7.0.
- Visual Studio 2022: MSVC v143 build tools (Latest), C++ ATL для v143 и Windows SDK 10.0.19041 или новее. Лучше одна установленная версия VS.
- Clang 19 или новее: с Servo 0.6 это обязательное требование на всех платформах ([August in Servo](https://servo.org/blog/2026/10/09/august-in-servo/)).
- Если сборка SpiderMonkey (mozjs) падает с ошибкой GetShortPathName, включаем короткие имена файлов 8.3; после обновления mozjs может понадобиться свежий MSVC.
- Какие ещё инструменты (Python через uv, GStreamer SDK) нужны при сборке именно через крейт с crates.io, а не через mach, фиксируем на первой неделе и записываем в `CONTRIBUTING.md`.

**Тестирование**

| Уровень | Что проверяем | Как |
| --- | --- | --- |
| Юнит-тесты core, storage, importer | вкладки, сессии, команды, форматы данных | тестовая реализация `Engine`, без Servo; быстро, в каждом коммите |
| Интеграция engine | адаптер делает то, что обещает трейт | настоящий Servo с `SoftwareRenderingContext`, локальные тестовые страницы |
| Дымовой прогон | реальные сайты не сломались после обновления движка | список сайтов, скриншоты через WebView::take\_screenshot, сравнение с прошлым выпуском |
| Сценарии интерфейса | горячие клавиши и поведение как в Firefox | чек-лист на каждую версию, позже автоматизация |

Совместимость со стандартами (WPT) не тестируем: это делает сам проект Servo.

**CI.** Windows-раннер GitHub Actions: форматирование, clippy, юнит-тесты на каждый коммит; полная сборка с Servo и дымовой прогон — раз в сутки и перед слиянием в main. Компиляция Servo долгая, поэтому кэш сборки (sccache) обязателен.

**Поставка**

- **Установщик:** на пользователя в %LOCALAPPDATA%, без прав администратора. Firefox использует NSIS; строим так же через cargo-packager: он собирает установщики NSIS (.exe) и MSI (WiX) и включает модуль обновления.
- **Состав:** бинарник, среда GStreamer для медиа, отдельный бинарник обновления, ресурсы локализации.
- **Подпись:** сертификат Authenticode для всех .exe и установщика, иначе предупреждения SmartScreen.
- **Обновления по модели Firefox:** каналы nightly / beta / release; фоновая проверка манифеста по HTTPS, загрузка пакета, проверка подписи (ed25519), установка при следующем запуске.
- **Отчёты о сбоях:** minidump-writer и rust-minidump — Rust-библиотеки, на которые Mozilla переводит систему отчётов о сбоях Firefox (клиент crash reporter переписан на Rust в Firefox 126); отправка только с согласия пользователя.

## Дорожная карта

Пять этапов идут строго по очереди; следующий начинается, только когда пройдены ворота предыдущего. Первый шаг — прототип: он раньше всего покажет, собирается ли Servo как библиотека под Windows и верна ли схема отрисовки с egui.

&#91;embedded content: дорожная карта · 5 этапов, 5 ворот\]

Состав каждой версии — в таблице функциональных требований. Ежемесячное обновление Servo идёт параллельно всем этапам.

## Риски и открытые вопросы

Главный риск — не в нашем коде, а в движке: часть популярных сайтов в Servo пока не работает, и никакой интерфейс этого не исправит.

| Риск | Чем грозит | Что делаем |
| --- | --- | --- |
| Совместимость движка: MSE нет вовсе, поэтому не работают YouTube и стриминг ([FOSS Linux, Servo 0.5](https://www.fosslinux.com/160711/what-is-servo-rust-browser-guide.htm)); IntersectionObserver и IndexedDB реализованы частично, ServiceWorker выключен ([Servo Book](https://book.servo.org/design-documentation/experimental-features.html)) | браузером нельзя пользоваться каждый день | честное позиционирование как ранней версии; решаем, какие экспериментальные возможности включить; страница совместимости; часть времени команды — на патчи в Servo |
| Ежемесячные ломающие изменения API Servo: в 0.7.0, например, убраны `Servo::focus`/`blur` и `notify_focus_changed` ([September in Servo](https://servo.org/blog/2026/11/05/september-in-servo/)) | судьба Verso: проект не успел за движком и закрылся ([Verso](https://github.com/versotile-org/verso/)) | весь servo заперт в крейте `engine`; обновление движка — плановая ежемесячная задача |
| Нет песочницы на Windows: многопроцессный режим есть, но песочница Servo реализована только для macOS и Linux ([sandboxing.rs](https://docs.rs/crate/servo-constellation/latest/source/sandboxing.rs)) | уязвимость в движке = доступ к профилю и паролям | процессы — для изоляции сбоев; до песочницы не рекомендовать браузер для банков; рассмотреть вклад в песочницу для Windows |
| application-services сделаны прежде всего для мобильных Firefox; криптография Sync идёт через NSS на C, а кросс-сборку NSS для Windows сама Mozilla называет непростой ([гайд по NSS](https://mozilla.github.io/application-services/book/howtos/upgrading-nss-guide.html)) | сложная сборка под Windows, исключение из «чистого Rust» | прототип сборки на первой неделе этапа 0.2; логины шифруем своим `EncryptorDecryptor` без NSS; если Sync не соберётся — порт нужных частей |
| Использование серверов Mozilla (аккаунт, Sync, Remote Settings) сторонним браузером: для входа в аккаунт нужен зарегистрированный у Mozilla OAuth-клиент | отказ в доступе или нарушение условий | до этапа 0.3 запросить у Mozilla условия; запасной вариант — свой сервер синхронизации на syncstorage-rs и, если понадобится, свой сервер аккаунтов ([документация syncstorage-rs](https://mozilla-services.github.io/syncstorage-rs/how-to/how-to-run-sync-server.html)) |
| Списки защиты от отслеживания Disconnect распространяются под CC BY-NC-SA 4.0 ([репозиторий](https://github.com/disconnectme/disconnect-tracking-protection/)) | коммерческий выпуск без лицензии Disconnect нарушит условия | для некоммерческого выпуска указать авторство; для коммерческого — договор с Disconnect или другие списки |
| GStreamer в установщике | размер установщика, лицензии кодеков | поставлять минимальный набор плагинов; лицензии проверить до первого публичного выпуска |
| Объём работ больше команды | проект застревает на полпути | версии с чёткими критериями готовности; каждая сама по себе полезна |

**Открытые вопросы**

- [ ] Название браузера (нужно для папки профиля, установщика и строк локализации).
- [ ] Принимаем ли два исключения из «чистого Rust»: NSS для криптографии Firefox Sync (логины можно шифровать и без него) и HTML/JS внутренних страниц?
- [ ] Синхронизация: серверы Mozilla или свой сервер?
- [ ] Команда и темп: сколько людей и часов в неделю. Без этого у дорожной карты нет дат.
- [ ] Лицензия нашего кода: предлагаю MPL 2.0, как у Firefox и Servo.
- [ ] Распространение: GitHub Releases или свой сайт; есть ли сертификат для подписи кода.

## Источники

**Servo**

- [Servo is now available on crates.io — блог Servo](https://servo.org/blog/2026/04/13/servo-0.1.0-release/)
- [August in Servo (Servo 0.6.0) — блог Servo](https://servo.org/blog/2026/10/09/august-in-servo/)
- [September in Servo (Servo 0.7.0) — блог Servo](https://servo.org/blog/2026/11/05/september-in-servo/)
- [Версии крейта servo — crates.io](https://crates.io/api/v1/crates/servo/versions)
- [Servo LTS releases — Servo Book](https://book.servo.org/embedding/lts-release.html)
- [Experimental Web Platform Features — Servo Book](https://book.servo.org/design-documentation/experimental-features.html)
- [Building for Windows — Servo Book](https://book.servo.org/building/windows.html)
- [servo/servo — README](https://github.com/servo/servo)
- [servo 0.5.0, lib.rs — docs.rs](https://docs.rs/crate/servo/0.5.0/source/lib.rs)
- [WebView — doc.servo.org](https://doc.servo.org/servo/struct.WebView.html)
- [WebViewDelegate — doc.servo.org](https://doc.servo.org/servo/trait.WebViewDelegate.html)
- [servo.rs: инициализация GStreamer — doc.servo.org](https://doc.servo.org/src/servo/servo.rs.html)
- [protocols/mod.rs: запрещённые схемы — docs.rs](https://docs.rs/servo-net/0.1.0/src/net/protocols/mod.rs.html)
- [sandboxing.rs, servo-constellation 0.7.0 — docs.rs](https://docs.rs/crate/servo-constellation/latest/source/sandboxing.rs)
- [Servo Book: встраивание и media-gstreamer](https://github.com/servo/book/pull/223/changes)
- [servo-media](https://github.com/eerii/media)
- [What Is Servo? (Servo 0.5) — FOSS Linux](https://www.fosslinux.com/160711/what-is-servo-rust-browser-guide.htm)
- [Verso (архив)](https://github.com/versotile-org/verso/)

**Firefox и Mozilla**

- [Keyboard shortcuts — Mozilla Support](https://support.mozilla.org/kb/Keyboard+shortcuts)
- [browser-sets.inc, Firefox 149 — Debian Sources](https://sources.debian.org/src/firefox/149.0-1/browser/base/content/browser-sets.inc)
- [What Windows 10 end of support means for Firefox users — Mozilla Support](https://support.mozilla.org/en-US/kb/firefox-support-windows-10-end-support)
- [mozilla/application-services](https://github.com/mozilla/application-services)
- [logins: модуль encryption — Application Services Book](https://mozilla.github.io/application-services/book/rust-docs/logins/encryption/index.html)
- [Guide to upgrading NSS — Application Services Book](https://mozilla.github.io/application-services/book/howtos/upgrading-nss-guide.html)
- [Компонент search, README](https://raw.githubusercontent.com/mozilla-firefox/firefox/main/third_party/application-services/components/search/README.md)
- [How to run a sync server — syncstorage-rs](https://mozilla-services.github.io/syncstorage-rs/how-to/how-to-run-sync-server.html)
- [disconnect-tracking-protection — Disconnect](https://github.com/disconnectme/disconnect-tracking-protection/)
- [fluent-bundle — docs.rs](https://docs.rs/crate/fluent-bundle/latest)
- [bergamot-translator, moz.yaml — Searchfox](https://arai.searchfox.org/firefox-main/source/toolkit/components/translations/bergamot-translator/moz.yaml)
- [ffext-manifest-schema: лицензии схем WebExtensions](https://awesome.ecosyste.ms/projects/github.com%2Fprsprsbk%2Fffext-manifest-schema)
- [A new crash reporter for Firefox — LWN](https://lwn.net/Articles/971006/)

**Инструменты**

- [cargo-packager — docs.rs](https://docs.rs/crate/cargo-packager/latest/)

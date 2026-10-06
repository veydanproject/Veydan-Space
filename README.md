# Veydan Space

> A workspace for multi-accounting.

> [!IMPORTANT]
> **Moving from Veydan Space 4? Read every step before you install version 5.**
> Version 5 does not open the local data of 4.x: your data moves only through
> sync. Once version 5 is installed, 4.x can no longer send your data anywhere.
>
> 1. **Turn sync on in 4.x:** Settings → Sync. No sync yet? On a computer no
>    cloud is needed: create an empty folder (for example `C:\VeydanSync` or
>    `~/VeydanSync`) and choose the type "Local folder" with it. On a phone
>    you need WebDAV or S3.
> 2. **Wait until sync finishes.**
> 3. **Remember your lock password:** version 5 opens the vault only with it.
>    If you use the messenger, save its key: Messenger → Settings → Export backup.
> 4. **Close 4.x and install version 5 over it** from
>    [the releases](https://github.com/veydanproject/Veydan-Space/releases/latest).
>    Do not uninstall 4.x first: an uninstaller can delete its data, and on a
>    phone it always does. On a computer, copy the data folder first as a
>    backup — Windows `%APPDATA%\net.veydan.space`, Linux
>    `~/.local/share/net.veydan.space`, macOS
>    `~/Library/Application Support/net.veydan.space`.
> 5. **In version 5, first of all connect sync to the same vault** (the same
>    folder, WebDAV or S3) with the same password: your data comes from
>    there. Then import the messenger key (Chat → Settings).
>
> Version 5 does not delete the files of 4.x: if something goes wrong,
> install 4.x again and it opens its data as before.

> [!IMPORTANT]
> **Переход с Veydan Space 4. Прочитайте все шаги до установки версии 5.**
> Версия 5 не открывает локальные данные 4.x: данные переносит только
> синхронизация. После установки версии 5 отправить данные из 4.x будет уже
> нечем.
>
> 1. **Включите синхронизацию в 4.x:** Настройки → Синхронизация.
>    Синхронизации ещё нет? На компьютере облако не нужно: создайте пустую
>    папку (например `C:\VeydanSync` или `~/VeydanSync`) и выберите тип
>    «Локальная папка» с этой папкой. На телефоне нужен WebDAV или S3.
> 2. **Дождитесь завершения синхронизации.**
> 3. **Запомните пароль блокировки:** без него версия 5 хранилище не
>    откроет. Если пользуетесь мессенджером, сохраните его ключ: Мессенджер →
>    Настройки → Экспорт копии.
> 4. **Закройте 4.x и установите версию 5 поверх неё** со
>    [страницы выпусков](https://github.com/veydanproject/Veydan-Space/releases/latest).
>    Не удаляйте 4.x заранее: деинсталлятор может стереть её данные, а на
>    телефоне стирает всегда. На компьютере сначала сделайте копию папки
>    данных — Windows `%APPDATA%\net.veydan.space`, Linux
>    `~/.local/share/net.veydan.space`, macOS
>    `~/Library/Application Support/net.veydan.space`.
> 5. **В версии 5 первым делом подключите синхронизацию к тому же
>    хранилищу** (та же папка, WebDAV или S3) с тем же паролем: данные придут
>    оттуда. Затем импортируйте ключ мессенджера (Чат → Настройки).
>
> Файлы 4.x версия 5 не удаляет: если что-то пойдёт не так, установите 4.x
> снова, и она откроет свои данные как раньше.

This repository holds the sources of **Veydan Space 5.0.4** and its releases.
It is a snapshot: the product is developed together with the other Veydan
apps, and each release is published here as one commit with the tag
`v<version>`. Nobody commits here by hand, so pull requests cannot be merged
in this repository; issues are welcome.

Built with **Tauri 2** (Rust) and **Svelte 5 / SvelteKit** (TypeScript).
Local-first: the data lives on your machine, with no telemetry.

## Build

Prerequisites: **Rust** (stable), **Node.js** with **pnpm**, and on Linux
WebKitGTK 4.1, GTK 3 and libsoup 3 (`scripts/dev.sh` can bootstrap a
self-contained toolchain and the system libraries into the project directory
without root).

```bash
pnpm --dir ui install --frozen-lockfile       # the UI project and the Tauri CLI, in ui/
scripts/dev.sh space                             # run in development
scripts/tauri.sh space build                     # native bundles under data/target/
```

The Tauri CLI is the one of the UI project (`ui/node_modules/.bin/tauri`)
and always runs from the root of the repository with `TAURI_APP_PATH`
pointing at the crate of the product and `TAURI_FRONTEND_PATH` at `ui/`;
`scripts/tauri.sh`, `scripts/dev.sh`, `scripts/update.sh` and
`scripts/android/*.sh` take the product as their first word and set them.

```bash
cargo check -p veydanspace                    # the crate of the product
node scripts/ui.mjs space build            # its UI, into data/build/space/ (the phone UI: data/build/space-android/)
node scripts/ui.mjs space svelte-check     # type check
node scripts/ui.mjs space vitest run       # unit tests of the UI
cargo test --workspace --all-targets        # tests of the crates
bash scripts/boundaries.sh                  # which crate and which UI folder may depend on which
```

## Layout

```
products.json     The product: its modules, their UI folders, routes and crates
apps/space/        The crate of the product: Tauri config, icons, gen/android, the module list
crates/           The platform (core, lock, sync, shell) and the modules of the product
ui/               SvelteKit frontend: src/lib/core and one folder per module
scripts/          Build, run and check scripts
data/             Everything the build makes (not in git): target/, build/, the toolchains
```

## License

Copyright © 2026 **Veydan Project**.

Veydan Space is **source-available** software, licensed under the
[PolyForm Perimeter License 1.0.1](https://polyformproject.org/licenses/perimeter/1.0.1):
see [`LICENSE`](LICENSE), a summary in [`LICENSE-SUMMARY.md`](LICENSE-SUMMARY.md)
and the licences of what it is built from in
[`THIRD-PARTY-LICENSES.md`](THIRD-PARTY-LICENSES.md).

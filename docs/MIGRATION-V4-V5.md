# Moving from Veydan Space 4 to version 5

**Read every step before you install version 5.**

Version 5 does not open the local data of 4.x: your data moves only through
sync. Once version 5 is installed, 4.x can no longer send your data anywhere.

1. **Turn sync on in 4.x:** Settings → Sync. No sync yet? On a computer no
   cloud is needed: create an empty folder (for example `C:\VeydanSync` or
   `~/VeydanSync`) and choose the type "Local folder" with it. On a phone
   you need WebDAV or S3.
2. **Wait until sync finishes.**
3. **Remember your lock password:** version 5 opens the vault only with it.
   If you use the messenger, save its key: Messenger → Settings → Export backup.
4. **Close 4.x and install version 5 over it** from
   [the releases](https://github.com/veydanproject/Veydan-Space/releases/latest).
   Do not uninstall 4.x first: an uninstaller can delete its data, and on a
   phone it always does. On a computer, copy the data folder first as a
   backup — Windows `%APPDATA%\net.veydan.space`, Linux
   `~/.local/share/net.veydan.space`, macOS
   `~/Library/Application Support/net.veydan.space`.
5. **In version 5, first of all connect sync to the same vault** (the same
   folder, WebDAV or S3) with the same password: your data comes from
   there. Then import the messenger key (Chat → Settings).

Version 5 does not delete the files of 4.x: if something goes wrong,
install 4.x again and it opens its data as before.

---

## Переход с Veydan Space 4 на версию 5

**Прочитайте все шаги до установки версии 5.**

Версия 5 не открывает локальные данные 4.x: данные переносит только
синхронизация. После установки версии 5 отправить данные из 4.x будет уже
нечем.

1. **Включите синхронизацию в 4.x:** Настройки → Синхронизация.
   Синхронизации ещё нет? На компьютере облако не нужно: создайте пустую
   папку (например `C:\VeydanSync` или `~/VeydanSync`) и выберите тип
   «Локальная папка» с этой папкой. На телефоне нужен WebDAV или S3.
2. **Дождитесь завершения синхронизации.**
3. **Запомните пароль блокировки:** без него версия 5 хранилище не
   откроет. Если пользуетесь мессенджером, сохраните его ключ: Мессенджер →
   Настройки → Экспорт копии.
4. **Закройте 4.x и установите версию 5 поверх неё** со
   [страницы выпусков](https://github.com/veydanproject/Veydan-Space/releases/latest).
   Не удаляйте 4.x заранее: деинсталлятор может стереть её данные, а на
   телефоне стирает всегда. На компьютере сначала сделайте копию папки
   данных — Windows `%APPDATA%\net.veydan.space`, Linux
   `~/.local/share/net.veydan.space`, macOS
   `~/Library/Application Support/net.veydan.space`.
5. **В версии 5 первым делом подключите синхронизацию к тому же
   хранилищу** (та же папка, WebDAV или S3) с тем же паролем: данные придут
   оттуда. Затем импортируйте ключ мессенджера (Чат → Настройки).

Файлы 4.x версия 5 не удаляет: если что-то пойдёт не так, установите 4.x
снова, и она откроет свои данные как раньше.

# 150. Пароль сетевого диска в связке ключей

ROADMAP: «Сеть, дальше: пароли в связке ключей (Secret Service)». TC: в FTP-соединении флажок
«Сохранить пароль» (шифрованно, по мастер-паролю).

## Как

- gvfs (1.54, `gvfskeyring.c`) сам ищет пароль в Secret Service перед вопросом — sftp, smb, ftp
  (`g_vfs_keyring_lookup_password`, схема `org.gnome.keyring.NetworkPassword`). Поиск — по
  подмножеству атрибутов, поэтому запись с теми же полями, что сохранил бы сам gvfs, находится:
  - sftp: `user` (если в адресе), `server`, `protocol=sftp`, `authtype=password`, `port` (если в адресе);
  - smb: `user`, `server`, `domain` (`smb://DOMAIN;user@…`), `protocol=smb`, `port`;
  - ftp: `user`, `server`, `protocol=ftp`, `port` кроме 21.
- Значит, хранить и подставлять сами не должны: после успешного Ctrl+F с паролем и флажком
  кладём запись (метка `user@server:port`, как у gvfs); дальше gvfs не спрашивает ни нас, ни «Файлы».
- Без glib: D-Bus через zbus (уже в сборке у libcosmic). `OpenSession("plain")`, `ReadAlias("default")`,
  `Unlock` (запрос связки — ждём `Completed`), `CreateItem(…, replace=true)`.
- Флажок «Запомнить пароль (связка ключей)» в Ctrl+F — для `sftp://`, `smb://`, `ftp://`; по
  умолчанию выключен (как в TC). Ошибка записи — сообщение, диск остаётся подключённым.

## Не делаем

- WebDAV: gvfs ищет его пароль по `object` = realm сервера — заранее не известен.
- Удаление записи при «Забыть адрес»: та же запись — у «Файлов» / Nautilus; удалять в «Пароли и ключи».

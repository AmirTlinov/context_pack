# recovery.md — типовые ошибки

Глобальный retry‑budget: максимум 3 попытки на тип операции, затем `BLOCKED`.

- `revision_conflict`:
  1) `get` → новый `revision`;
  2) повторить мутацию;
  3) после 3 попыток → `BLOCKED`.

- `not_found`:
  1) проверить `id|name`;
  2) `list` → выбрать корректный;
  3) если нет — `create` (если допустимо scope).

- `expired`:
  1) найти по `name`;
  2) если удалён — `create` новый;
  3) вернуть новый `pack_id`.

- `stale_ref`:
  1) исправить `path/lines` или `delete_ref`;
  2) повторить QA;
  3) если не снимается → `draft + gaps`.

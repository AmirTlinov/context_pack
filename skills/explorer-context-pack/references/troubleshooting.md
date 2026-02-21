# troubleshooting.md — частые проблемы

- `revision_conflict` → перечитать pack (`get`), обновить `expected_revision`.
- `stale_ref` → поправить `path/lines` или удалить ref.
- `not_found` → pack мог истечь по TTL.
- `tool output too large` → разбить пакет на меньшие секции (output ограничен по размеру).

# Anti-patterns (не делать)

## Contract violations

1. Использовать v2 действия: `create`, `upsert_*`, `set_*`, `touch_ttl`, `output get`.
2. Использовать `delete_pack` вместо `input {"action":"delete"}`.
3. Передавать `mode/cursor/match/format` в `output.read`.

## Consistency violations

4. Писать update без `expected_revision`.
5. Делать partial snapshot (`input.write`) и случайно удалять секции.
6. Финализировать без `validate_only=true` precheck.
7. Игнорировать `invalid_refs`/`stale ref` перед finalize.

## Handoff / collaboration mistakes

8. Передавать pack по `name`, а не по точному `pack_id`.
9. Не фиксировать `review URL + verdict` в completion.
10. Хранить факты в чате вместо refs/sections в пакете.

## Freshness mistakes

11. Игнорировать `expiring_soon` и терять контекст по TTL.
12. Не проверять `freshness=expired`, когда pack «исчез» из default list.

## Search quality mistakes

13. Ожидать regex-поиск от `contains`.
14. Не дублировать finding-токены (`FND-*`) в `ref.title/ref.why`.

## Smell: переусложнение

15. Секции с длинным narrative без anchors.
16. Десятки неструктурированных refs без группировки `group`.

Если замечен любой пункт выше — остановись, вернись к
`00-v3-contract-cheatsheet.md` и приведи pack к v3 discipline.

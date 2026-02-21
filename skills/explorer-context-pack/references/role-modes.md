# role-modes.md — режимы ролей

### Explorer (быстрый)
- Узкий scope, минимум шагов.
- Обычно 3–10 anchors.

### Deep Explorer (глубокий)
- Широкое покрытие + edge‑cases.
- Обычно 10+ anchors, при необходимости диаграммы.
- Обязателен блок risks/gaps/next checks.

### Reviewer
- Только узкий валидный scope; иначе `BLOCKED`.
- Findings обязаны иметь REF + evidence + fix + validation.

## Batch‑ритм (скорость без потери качества)

Разрешено собирать несколько якорей за цикл:
1) `get revision`
2) серия `upsert_ref`
3) промежуточный `output get`
4) следующая серия

# План: Engagement Posture — обязательное интервью о позе вовлечения и posture-gated guardrails

> Статус: утверждён к реализации, не начат. Решения по развилкам зафиксированы 2026-09-08.
> Область: только Infrastructure-Creator (Kit 1). Готовые издания (Kit 2) — вне v1.
> Характер: слой секьюрности поверх существующей механики; не структурное изменение.

## Проблема

Генератор задаёт один обязательный вопрос (выбор AI-инструмента) и выводит
guardrails только из фактов проекта (evidence-gated). Решения *команды* —
видима ли AI-инфраструктура клиенту, кто коммитит, что агенту можно делать
самому — сейчас нигде не спрашиваются и не влияют на сгенерированные хуки и
политику. На аутстаффе это прямой риск для разработчика.

## Зафиксированные решения

| Развилка | Решение |
| --- | --- |
| Невидимый режим (P1) | Реализуется через `.git/info/exclude` (не `.gitignore` — он tracked и сам выдаёт факт установки). В невидимом режиме запрещены любые мутации tracked-файлов клиента, включая консервативные мёржи в `.gitignore`/`.gitattributes`/`AGENTS.md`. Побочный эффект фиксируется в тексте вопроса: `memory-bank/`/`project-brain/` перестают быть shared-памятью команды — одиночный режим. |
| Машиночитаемость | Профиль (новая секция) + `.infra-manifest.json`. Plan schema остаётся 1.6 — без бампа. |
| Формат вопросов | Фиксированные варианты + Other. Свободная проза не парсится хуками. |
| Умолчание | Fail-closed: нет ответа → строжайшая поза (человек коммитит, только черновики, рискованные операции руками). |
| Fan-out ответов | Каждый posture-вопрос задаётся **отдельно**, никогда не внутри другого вопроса (полевой инцидент прототипа: свёрнутый вопрос о commit-policy доехал до 1 файла из 6). Для каждого ответа — явная таблица fan-out «ответ → файлы, куда он применяется»; конформанс-гейт шага 6 проверяет каждую строку этой таблицы, а не факт наличия ответа. |

## Блок вопросов (обязательный, по образцу AI-tool вопроса)

Машиночитаемые строки ответов в `clarifying-interview-answers.md`:

```
posture.visibility: agreed-visible | invisible-local
posture.commits: agent-commits | human-only | agent-commits-human-pushes
posture.external_actions: agent-may-publish | drafts-only
posture.automation: [migrations, deploys, dependency-updates, db-schema]   # что разрешено агенту
posture.data_boundary: network-allowed | local-only
```

Provenance каждого ответа — `interview answer`, как у существующих ответов.

## Шаги реализации (канон — `Infrastructure-Creator/.agents/skills/`)

1. **`clarifying-interview`** — второй обязательный блок «Engagement Posture»
   (P1–P5), фиксированные варианты + Other, машиночитаемая запись, fail-closed
   умолчания.
2. **`profile-synthesizer`** — новая секция профиля «Engagement Posture»;
   жёсткий стоп при отсутствии ответов (тот же паттерн, что для editions:
   «never assume»).
3. **`hook-forge`** — второй класс правил: **posture-gated** рядом с
   существующими evidence-gated. `human-only` → блок `git commit`/`git push`;
   `drafts-only` → блок обнаруженных publish-CLI (`gh pr comment`, `glab` —
   только если инструмент реально найден); automation-список → блоки
   соответствующих команд, даже «безопасных» (`composer update`,
   `artisan migrate` без `:fresh`). В логе каждое правило цитирует либо
   evidence, либо ответ интервью — раздельно, не смешивая.
4. **`policy-forge`** — секция MUST/MUST NOT в генерируемом `AGENTS.md`
   таргета, выведенная из позы (кто коммитит; черновики вместо публикаций;
   что только руками).
5. **`infra-generate` (публикация)** — invisible-ветка: запись всех
   публикуемых путей в `.git/info/exclude` таргета, подавление мёржей в
   tracked-файлы, `.infra-manifest.json` тоже в exclude.
6. **`bootstrap-verifier`** — конформанс-гейт «заявленная поза ↔ реальные
   артефакты»: `human-only` без блока на `git push` в `bash-validator.sh` —
   фейл; invisible без полного покрытия exclude — фейл. Поза не может быть
   декоративной.
7. **Манифест + `infra-update`** — поза записывается в
   `.infra-manifest.json`; при update не переспрашивается, но изменение позы —
   diff, требующий явного решения (как правки файлов команды).
8. **Тесты** — `Infrastructure-Creator/tests/`: расширить
   `test_scan_contracts` (контракт интервью), `test_hooks` (posture-правила),
   фикстуры invisible-режима.
9. **Зеркала + changelog** —
   `python3 scripts/build_mirrors.py --write --edition Infrastructure-Creator`,
   запись в `Infrastructure-Creator/CHANGELOG.md`.

## Вне объёма v1

- Kit 2 (у инсталлятора нет интервью-фазы; posture-флаги инсталлятору — отдельный шаг).
- Guard-hook для личного рабочего окружения инженера (wrapper workspace:
  личная папка-обёртка вне клиентского репозитория, fail-closed хук против
  утечки AI-конфига в клиентский index) — отдельная работа, планом не покрыта.
- Бамп plan schema.

## Синергия

`posture.external_actions: drafts-only` — тот же паттерн, что «Черновики»
(фаза 8) в плане усиления flow-feature: политика из этого плана, артефакты
(`mr.md`, `tracker-comment.md`) — из того.

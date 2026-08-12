> Источник: BAUMAS-60 (2026-08-08), отредактировано по ревью той же датой:
> четыре фактических уточнения (cl100k vs stdlib-CI, статус IC-форджей,
> аудит mailbox, реализуемость E2E) и добавлен блок находок боевого ревью.
> Язык оригинала сохранён.

# Backlog поверх perf/context-economy

Практичный backlog поверх того, что уже на ветке (Stages A–D,
economy-правки, harness). Не «всё подряд», а то, что ещё даёт смысл по
дизайну и измерениям.

## Уже закрыто (не дублировать)

- Оркестрация A–C: `/flow-feature`, `/flow-review`, channel, write-lock,
  SubagentStop observer
- Stage D: `harness/` (fleet-review) — см. [../harness/README.md](../harness/README.md)
- Context economy: `.gitattributes` на зеркала, Cursor capsule без per-turn
  invalidators, MCP-секция в edition README, `cost_attribution.py`, пороги в
  [OPERATIONS.md](OPERATIONS.md)
- Лимиты payload для browser verification; калиброванные категории
  context-budget; lint-регрессия против Cursor invalidators; атомарный захват
  write-lock; разбор только первого блока frontmatter в subagent gate
- Subagent gate в форджах Infrastructure-Creator (hook-forge генерирует семь
  хуков с полным вайрингом; см. §2 — что ещё НЕ прокинуто)

## 1. Высокий ROI, «домашние» правки (stdlib)

| Что | Зачем |
|---|---|
| Документировать OTel env vars в OPERATIONS.md | Опциональный measurement layer; primary attribution уже в transcript'ах. |
| Интерактивная калибровка /context + /doctor | Уточнить unit skill-listing budget (chars vs tokens) и фактический MCP-prefix — влияет на все бюджеты. |

## 1b. Находки боевого ревью

Две подтверждённые находки — атомарный захват write-замка и разбор только
первого блока фронтматтера — уже исправлены и перенесены в список закрытого
выше. Открыты только принятые условные риски:

Условные (по требованию, TTL страхует; не чинить без нужды):

- Захват замка на PreToolUse без гарантии SubagentStop (отказ после гейта →
  замок до TTL) — принятый компромисс.
- /tmp-hardening замков (symlink/predictable path) — паттерн идентичен
  loop-detection; менять только вместе с ним.
- Same-agent параллельные инстансы делят замок — сознательное допущение
  retry-сценария.
- Payload без имени агента оставляет замок до TTL — release без имени
  невозможен by design.

## 2. Оркестрация: следующий слой поверх A–D

Новые flows (дёшево, тот же substrate):

- `/flow-hotfix` — debugger → coder → test → review (короткий path)
- `/flow-research` — fan-out read-only (mapper ∥ researcher ∥ security) → synthesis
- `/flow-migration` / `/flow-release` — для массовых/релизных сценариев

Infrastructure-Creator (правка статуса: **гейт уже прокинут** — hook-forge
генерирует `subagent-gate.sh` с полным per-edition вайрингом; реальный гэп
другой):

- command-forge / hook-forge / agent-forge должны генерировать **flows,
  channel (msg-*), SubagentStop observer и `writes: true` разметку** для
  project-specific accelerator. Сейчас agent-forge не эмитит `writes:`
  (сериализация в сгенерированных акселераторах спит), а `/flow-*` и
  msg-dispatch не генерируются вовсе — generated отстаёт от hand-built
  ровно на Stage A–C.

Harness (Stage D+):

- Второй граф: mass-migration, scheduled research, nightly security scan
- Адаптер Claude Agent SDK (предпочтительнее сырого CLI для Python)
- Метрики → cost_attribution / отчёт по total_cost_usd
- Cursor node не трогать (hangs)

Codex multi-agent — только если появится реальная нужда; дизайн намеренно
оставил off.

## 3. Context / memory (из deferred specs)

| Идея | Условие |
|---|---|
| Local embeddings / hybrid search | Только если golden-query покажет, что FTS5 recall недостаточен. Сейчас snippet 32 FTS-токена, summary 8× vs content — «богатая карта» почти не доезжает. |
| MCP-адаптер к context.py | Только если клиентам нужен protocol discovery, а не CLI через skill. |
| context.py memory native | Отложено: либо дубль orchestration, либо model integration. |
| Automatic semantic/procedural authoring | Вне scope: runtime умеет узкую, явно настроенную auto-promotion терминальных verified records, но не свободное автоматическое авторство semantic/procedural knowledge. |
| Hooks/background sync unattended | Когда реально понадобится capture без сессии. |

Mailbox: **аудит уже есть** — журнал в `control/` (git-tracked), пишется под
общим mutation-lock, валидируется построчно со строгим seq; git-история
append-only файла и есть аудит-трейл. Governed CAS на сообщение добавил бы
ревизии перезаписи, которой у канала сознательно нет. Не ужесточать, пока не
появится требование перезаписи/ревизий сообщений.

## 4. Продуктовые расширения акселератора

- Новые framework-skills под реальные gaps команд (Livewire/Inertia глубже,
  Laravel 13, Symfony UX, multi-tenant, Outbox/CQRS) — только по evidence,
  не «потому что модно».
- Council / multi-perspective review как flow (уже есть council agent —
  связать в `/flow-*`).
- Economy 2 measurement: cost_attribution на потребляющем PHP-проекте с
  установленной edition — сейчас вся экономика измерена на monorepo.
- Per-skill cost → prune/merge skills: research: 7 % skill-attributed,
  engineering:code-review ~6 %; после attribution — сократить/объединить
  тяжёлые skill'ы.
- Опциональный Context7 в adoption guide (не ship): низкий schema-cost,
  закрывает «версия фреймворка vs cutoff».
- Scoped GitHub MCP для consumer-проектов (не monorepo: там gh без prefix).

## 5. Качество / DX

- Golden retrieval fixtures расширить (сейчас один retrieval-golden.json) —
  регрессии recall после любых правок engine.
- E2E smoke (правка реализуемости): **stdlib-половина в CI монорепо**
  (install → channel roundtrip → capsule validate → журнал; частично уже
  покрыто test_channel), **полный flow-путь и harness dry-run — вне CI
  монорепо** (LLM-токены и langgraph-зависимость соответственно): отдельный
  workflow в каталоге harness или dev-скрипт.
- Install matrix уже 9-way; добавить проверку inventory на flows/hooks
  Stage C.
- Skill body size CI (см. §1) — режет bloat на входе.
- Parité IC ↔ hand editions checklist: gate, flows, writes-lock, MCP
  section, capsule delivery.

## 6. Сознательно не делать (и почему)

| Кандидат | Почему нет |
|---|---|
| code2prompt / repomix / packers в runtime | Uncached input ≈ 0.66 % spend; packers бьют мимо. CLI только как offline-meter. |
| LangGraph внутри edition | Stdlib-only, no-daemon, README non-feature; harness — companion. |
| Vector DB / embeddings по умолчанию | Install → network, fail-hard; FTS5 сначала. |
| A2A / agent bus / daemon | Всё остаётся repo-local, files + CLI. |
| Orchestrator-subagent | Main conversation лучше (depth, permissions, checkpoints). |
| Unscoped GitHub MCP / Playwright all-caps | 8–11× prefix акселератора. |

## Рекомендуемый порядок (если выбирать 1–2 спринта)

1. Документировать optional OTel env vars и выполнить интерактивную
   калибровку `/context` + `/doctor`.
2. 1–2 новых flow + IC forges под Stage A–C — product parity generated vs
   hand-built.
3. Economy-2 attribution на реальном PHP-проекте → prune skills.
4. Harness graph #2 только при реальном unattended pipeline.

---

Коротко: фундамент (policy, memory, gates, flows, harness, economy) уже
плотный. Дальше выигрыш не в «ещё один framework skill», а в:

1. резать payload/budget (browser, skill bodies, budgets),
2. догнать IC до hand-built orchestration (Stage A–C в форджах),
3. измерить economy на consumer-проектах,
4. добавлять flows/graphs только под реальные multi-step сценарии.

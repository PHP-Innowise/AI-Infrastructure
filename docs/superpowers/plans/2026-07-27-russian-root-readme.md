# Russian Root README Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Полностью переписать корневой `README.md` на русском языке и подробно, но без неподтверждённых обещаний, объяснить устройство акселераторов и локального Context Engine.

**Architecture:** Изменение ограничено одним пользовательским документом. README сохраняет роль общей точки входа, ссылается на README редакций за деталями и содержит самодостаточное описание четырёхслойной локальной памяти, её CLI, безопасности, ограничений и проверенного Bauherrenmappe-сценария.

**Tech Stack:** Markdown, Python 3.9+, SQLite FTS5 CLI, Git.

## Global Constraints

- Изменять только корневой `README.md`; код, README редакций и Context Engine не менять.
- Весь объясняющий текст должен быть на русском; английские имена файлов, директорий, команд, слоёв и инструментов сохраняются как фактические идентификаторы.
- Не заявлять об автоматической подстановке контекста, embeddings, vector search, MCP, LangGraph или центральном сервисе памяти.
- Репозиторные код, конфигурация, тесты, спецификации и политики остаются источниками истины.
- Не добавлять зависимости, генераторы документации или новые вспомогательные скрипты.
- Не изменять и не добавлять в Git существующие `.langgraph_api/` и `ai_infrastructure_langgraph.egg-info/`.

---

### Task 1: Переписать и проверить корневой README

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: фактическая структура `Laravel/`, `Symfony/`, `PHP Core/`, `Infrastructure-Creator/`; CLI `memory-bank/scripts/context.py`; подтверждённые тесты и Bauherrenmappe black-box результаты.
- Produces: русскоязычная точка входа в репозиторий с рабочими ссылками и командами.

- [ ] **Step 1: Зафиксировать исходное несоответствие**

Проверить, что текущий документ всё ещё англоязычный и не содержит
запланированной русской структуры:

```bash
rg -n '^# PHP AI Accelerators$|^## Which One Do I Use\\?$|^## Local Context Engine$' README.md
rg -n '^## Как выбрать редакцию\\?$|^## Локальный Context Engine$|^## Безопасность и ограничения$' README.md
```

Ожидаемый результат: первая команда находит английские заголовки, вторая
завершается без совпадений.

- [ ] **Step 2: Переписать вводную часть и навигацию**

Заменить английскую вводную следующей русской структурой:

```text
# PHP AI Accelerators
## Что находится в репозитории
## Как выбрать редакцию
## Как подключить акселератор к проекту
## Общая архитектура: Command → Agent → Skill
## Поддерживаемые AI-инструменты
## Чем отличаются редакции
```

В этом блоке:

- объяснить назначение `Laravel/`, `Symfony/`, `PHP Core/` и
  `Infrastructure-Creator/`;
- сохранить рабочие относительные ссылки на локальные README;
- объяснить два способа подключения: открыть редакцию как workspace root или
  скопировать её содержимое в корень проекта;
- явно сказать, что открытие корня монорепозитория не активирует вложенную
  редакцию автоматически;
- сохранить различия Claude Code, Cursor и Codex;
- перечислить только реально существующие stack-specific возможности.

- [ ] **Step 3: Добавить подробное описание Memory Bank**

Создать разделы:

```text
## Memory Bank
### Что хранится в общей памяти
### Источники истины и provenance
### Локальная база данных
```

Объяснить, что committed chunks предназначены для проверяемых долговременных
знаний, а `memory-bank/local/context.db` хранит производный индекс и локальные
working/episodic-записи. Указать, что индекс документов пересоздаётся, а
локальные задачи и эпизоды при удалении БД теряются.

- [ ] **Step 4: Подробно описать четыре слоя Context Engine**

Добавить таблицу с точными слоями и источниками:

| Слой | Содержимое |
| --- | --- |
| `working` | Явное состояние активной задачи по `task-id` |
| `procedural` | `AGENTS.md`, `CLAUDE.md`, локальные skills и правила |
| `semantic` | `README`, `docs/`, `specs/`, активные chunks, tasks и epics |
| `episodic` | `CHANGELOG.md` и локальные эпизоды завершённых задач |

Указать, что классификация durable-источников выполняется автоматически, а
`working` создаётся только явной командой. Объяснить ограниченную выборку
отдельно по каждому долговременному слою и обязательную проверку найденного
контекста по репозиторному источнику.

- [ ] **Step 5: Добавить полный CLI workflow**

Добавить команды, запускаемые из корня выбранной редакции или потребляющего
проекта:

```bash
python3 memory-bank/scripts/context.py index
python3 memory-bank/scripts/context.py start \
  --task-id BAUMAS-133 \
  --goal "Проверить инвалидирование других сессий после смены пароля" \
  --file src/GraphQL/Resolver/ChangePasswordResolver.php
python3 memory-bank/scripts/context.py update \
  --task-id BAUMAS-133 \
  --progress "Регрессионный тест с двумя сессиями проходит" \
  --next-step "Проверить старый remember-me cookie" \
  --file tests/Integration/GraphQL/ChangePasswordTest.php
python3 memory-bank/scripts/context.py context \
  "PdoSessionHandler password sessions" \
  --task-id BAUMAS-133
python3 memory-bank/scripts/context.py complete \
  --task-id BAUMAS-133 \
  --outcome "Другие сессии и устаревшие remember-me cookies инвалидируются" \
  --verification "ChangePasswordTest passed"
python3 memory-bank/scripts/context.py search BAUMAS-133
python3 memory-bank/scripts/context.py status
```

После основного примера кратко объяснить:

- источник `task-id`: ticket, branch или описательный slug;
- одновременную работу нескольких задач с разными ID;
- `get --task-id`, `clear --task-id`, совместимый `record`;
- `--json`, `--limit`, `--layer`, повторяемые `--file`, `--source`,
  `--next-step` и `--verification`;
- атомарный `complete` и сериализацию конкурентных `start`/`update`.

- [ ] **Step 6: Описать безопасность и честные ограничения**

Добавить отдельный раздел, который:

- запрещает raw conversations, prompts, responses, logs, credentials, secrets,
  customer data и personal data;
- объясняет, что CLI отклоняет значения, похожие на известные виды секретов,
  не повторяя найденное значение в ошибке;
- подчёркивает локальность и Git-ignore базы;
- явно перечисляет отсутствующие возможности: автоматическая per-request
  injection, embeddings, vector search, MCP, LangGraph и центральный memory
  service.

- [ ] **Step 7: Зафиксировать проверенный Bauherrenmappe-сценарий**

Описать только подтверждённые результаты:

```text
documents=4
procedural=1
semantic=3
episodic documents=0
completed local episodes=1
working=0
Git status unchanged
temporary external database removed
```

Уточнить, что это black-box проверка одной локальной копии проекта, а не
обещание универсальной автоматической интеграции.

- [ ] **Step 8: Переписать Infrastructure Creator и contributing-разделы**

Перевести оставшиеся разделы, сохранив:

- отличие генератора от готовых редакций;
- поток `infra-scan → review → infra-generate`;
- вариант `infra-build`;
- правило держать генератор вне целевого проекта;
- tool-mirroring и framework-specific границы при внесении изменений.

- [ ] **Step 9: Проверить структуру, команды и ссылки**

Проверить заголовки и обязательные фактические ограничения:

```bash
rg -n '^## (Что находится в репозитории|Как выбрать редакцию|Memory Bank|Локальный Context Engine|Безопасность и ограничения|Проверка на Bauherrenmappe|Infrastructure Creator|Как внести изменения)' README.md
rg -n 'start → update → context → complete|BEGIN IMMEDIATE|automatic|embeddings|vector search|MCP|LangGraph|центральн' README.md
python3 Symfony/memory-bank/scripts/context.py --help
```

Проверить все локальные Markdown-ссылки без добавления нового скрипта:

```bash
python3 -c 'import pathlib,re; p=pathlib.Path("README.md"); links=re.findall(r"\[[^]]+\]\((?!https?://|#)([^)#]+)", p.read_text()); missing=[x for x in links if not (p.parent / x.replace("%20", " ")).exists()]; assert not missing, missing'
```

- [ ] **Step 10: Запустить полную проверку**

```bash
for edition in Laravel "PHP Core" Symfony; do
  python3 -m unittest discover -s "$edition/memory-bank/tests" -q
  python3 "$edition/memory-bank/scripts/validate.py"
done
git diff --check
git status --short
```

Ожидаемый результат: 45 тестов проходят в каждой редакции, все три валидатора
успешны, whitespace errors отсутствуют, а из tracked-файлов изменён только
`README.md`.

- [ ] **Step 11: Закоммитить README**

```bash
git add -- README.md
git diff --cached --check
git commit -m "docs: write detailed Russian root readme"
```

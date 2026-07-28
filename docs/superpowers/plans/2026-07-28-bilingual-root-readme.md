# Bilingual Root README Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Создать полные английскую и русскую версии корневой документации в `README_EN.md` и `README_RU.md`, оставив `README.md` коротким выбором языка.

**Architecture:** Текущий проверенный русский README становится основой `README_RU.md`; `README_EN.md` содержит эквивалентный естественный английский текст. Корневой `README.md` остаётся стабильной GitHub-точкой входа и ссылается на оба языка.

**Tech Stack:** Markdown, Python 3.9+, Git.

## Global Constraints

- `README.md` содержит только заголовок проекта и ссылки на `README_EN.md` и `README_RU.md`.
- `README_EN.md` содержит полную английскую документацию.
- `README_RU.md` сохраняет полную текущую русскую документацию.
- В обеих полных версиях должен быть переключатель языка перед основным заголовком.
- Команды, пути, версии, числовые результаты, ограничения и границы Context Engine должны совпадать между языками.
- Английский текст должен быть естественным техническим английским, а не дословным машинным переводом.
- Код, README редакций, Context Engine и changelog не меняются.
- Не добавлять генератор переводов, синхронизирующий скрипт или новую зависимость.
- Не изменять и не добавлять в Git существующие `.langgraph_api/` и `ai_infrastructure_langgraph.egg-info/`.

---

### Task 1: Создать две полные языковые версии и корневой переключатель

**Files:**
- Modify: `README.md`
- Create: `README_EN.md`
- Create: `README_RU.md`

**Interfaces:**
- Consumes: текущий подробный русский `README.md`, edition README, фактический Context Engine CLI.
- Produces: три взаимосвязанных Markdown-файла с рабочими локальными ссылками.

- [ ] **Step 1: Зафиксировать исходное состояние**

```bash
test ! -e README_EN.md
test ! -e README_RU.md
rg -n '^## (Что находится в репозитории|Memory Bank|Локальный Context Engine|Infrastructure Creator)' README.md
```

Ожидаемый результат: языковых файлов ещё нет, а полный русский текст находится
в `README.md`.

- [ ] **Step 2: Создать полную русскую версию**

Скопировать текущее содержимое `README.md` в `README_RU.md` без смысловых
изменений. Перед `# PHP AI Accelerators` добавить:

```markdown
[English](README_EN.md) | Русский
```

Не менять команды, ссылки, проверенные результаты или формулировки границ
Context Engine.

- [ ] **Step 3: Создать полную английскую версию**

Создать `README_EN.md` с переключателем:

```markdown
English | [Русский](README_RU.md)

# PHP AI Accelerators
```

Использовать такой набор основных заголовков в том же порядке:

```text
## What Is in This Repository
## Which Edition Should You Choose?
## How to Add an Accelerator to a Project
## Shared Architecture: Command → Agent → Skill
## Supported AI Tools
## How the Editions Differ
## Memory Bank
## Local Context Engine
## Security and Limitations
## Bauherrenmappe Verification
## Infrastructure Creator
## Contributing
```

Перевести весь объясняющий текст. Сохранить без смысловых изменений:

- таблицу Laravel, Symfony и PHP Core с версиями PHP;
- два способа подключения редакции и предупреждение о корне монорепозитория;
- различия Claude Code, Cursor и Codex;
- правило временных `tasks/` и постоянных `specs/`;
- Memory Bank authority/provenance и локальность SQLite;
- четыре слоя `working`, `procedural`, `semantic`, `episodic`;
- все команды примера и searchable `BAUMAS-133` summary;
- различие `context` и `search`;
- атомарность `complete` и `BEGIN IMMEDIATE` для конкурентных изменений;
- запрет на секреты и сырые диалоги/логи;
- отсутствие automatic per-request injection, embeddings, vector search, MCP,
  LangGraph и центрального memory service;
- точные Bauherrenmappe-результаты;
- гарантии `infra-scan` и `infra-generate`;
- ссылки на README и Memory Bank всех редакций.

Текстовые значения CLI-примера перевести на английский, не меняя команды,
флаги, `task-id`, пути файлов и порядок жизненного цикла.

- [ ] **Step 4: Заменить корневой README переключателем**

Записать в `README.md` только:

```markdown
# PHP AI Accelerators

[English](README_EN.md) | [Русский](README_RU.md)
```

- [ ] **Step 5: Проверить навигацию и локальные ссылки**

```bash
python3 - <<'PY'
from pathlib import Path
import re

for name in ("README.md", "README_EN.md", "README_RU.md"):
    path = Path(name)
    text = path.read_text(encoding="utf-8")
    links = re.findall(r"\[[^]]+\]\((?!https?://|#)([^)#]+)", text)
    missing = [
        link
        for link in links
        if not (path.parent / link.replace("%20", " ")).exists()
    ]
    assert not missing, (name, missing)

assert "[English](README_EN.md)" in Path("README.md").read_text()
assert "[Русский](README_RU.md)" in Path("README.md").read_text()
assert "[Русский](README_RU.md)" in Path("README_EN.md").read_text()
assert "[English](README_EN.md)" in Path("README_RU.md").read_text()
print("language navigation and local links: OK")
PY
```

- [ ] **Step 6: Проверить структурную и фактическую эквивалентность**

```bash
python3 - <<'PY'
from pathlib import Path

english = Path("README_EN.md").read_text(encoding="utf-8")
russian = Path("README_RU.md").read_text(encoding="utf-8")

english_headings = [
    "## What Is in This Repository",
    "## Which Edition Should You Choose?",
    "## How to Add an Accelerator to a Project",
    "## Shared Architecture: Command → Agent → Skill",
    "## Supported AI Tools",
    "## How the Editions Differ",
    "## Memory Bank",
    "## Local Context Engine",
    "## Security and Limitations",
    "## Bauherrenmappe Verification",
    "## Infrastructure Creator",
    "## Contributing",
]
russian_headings = [
    "## Что находится в репозитории",
    "## Как выбрать редакцию",
    "## Как подключить акселератор к проекту",
    "## Общая архитектура: Command → Agent → Skill",
    "## Поддерживаемые AI-инструменты",
    "## Чем отличаются редакции",
    "## Memory Bank",
    "## Локальный Context Engine",
    "## Безопасность и ограничения",
    "## Проверка на Bauherrenmappe",
    "## Infrastructure Creator",
    "## Как внести изменения",
]

assert all(heading in english for heading in english_headings)
assert all(heading in russian for heading in russian_headings)

required_literals = [
    "Laravel 12 / 13",
    "Symfony 7.4 LTS",
    "Symfony 8.1",
    "PHP 8.2+",
    "BAUMAS-133",
    "start → update → context → complete",
    "BEGIN IMMEDIATE",
    "documents=4",
    "procedural=1",
    "semantic=3",
    "episodic documents=0",
    "completed local episodes=1",
    "working=0",
    "Git status unchanged",
    "temporary external database removed",
    "infra-scan",
    "infra-generate",
]
for literal in required_literals:
    assert literal in english, ("English", literal)
    assert literal in russian, ("Russian", literal)

required_commands = (
    "index", "start", "update", "context", "complete", "search", "status"
)
for command in required_commands:
    needle = f"memory-bank/scripts/context.py {command}"
    assert needle in english, ("English", needle)
    assert needle in russian, ("Russian", needle)

print("structure and factual parity: OK")
PY
```

- [ ] **Step 7: Проверить CLI и полный набор тестов**

```bash
python3 Symfony/memory-bank/scripts/context.py --help
for edition in Laravel "PHP Core" Symfony; do
  python3 -m unittest discover -s "$edition/memory-bank/tests" -q
  python3 "$edition/memory-bank/scripts/validate.py"
done
git diff --check
git status --short
```

Ожидаемый результат: CLI содержит все документированные команды, по 45 тестов
проходят в каждой редакции, все три валидатора успешны, whitespace errors
отсутствуют, а из tracked-файлов изменены только три README.

- [ ] **Step 8: Закоммитить языковые версии**

```bash
git add -- README.md README_EN.md README_RU.md
git diff --cached --check
git commit -m "docs: add English and Russian readmes"
```

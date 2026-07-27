# PHP AI Accelerators

Набор готовых акселераторов для AI-агентов в PHP-проектах и генератор для
создания такого акселератора по фактической структуре отдельного проекта.
Каждая редакция объединяет правила, команды, агентов, skills, проверки
качества и соглашения по документации. Она не заменяет код, конфигурацию,
тесты и спецификации проекта.

## Что находится в репозитории

~~~text
AI-Infrastructure/
├── Laravel/                  # готовая Laravel-редакция
├── Symfony/                  # готовая Symfony-редакция
├── PHP Core/                 # готовая редакция для нативного PHP
└── Infrastructure-Creator/   # генератор под конкретный проект
~~~

- [Laravel/](Laravel/README.md) — готовая редакция для Laravel: Eloquent,
  очереди, события, уведомления, Filament и разработка пакетов.
- [Symfony/](Symfony/README.md) — готовая редакция для Symfony: практические
  границы Controller → Service → Repository, Doctrine, Messenger, API
  Platform, voters, Forms и Symfony UX.
- [PHP Core/](PHP%20Core/README.md) — нейтральная к фреймворку основа для
  Composer + PSR-проектов, PDO и явных границ приложения.
- [Infrastructure-Creator/](Infrastructure-Creator/README.md) — не готовая
  редакция для копирования, а генератор, который исследует целевой проект и
  создаёт подходящий workflow layer.

Первые три каталога — самостоятельные готовые редакции.
Infrastructure-Creator/ решает другую задачу: создаёт новую редакцию по
составу, интеграциям, архитектуре и CI/CD указанного PHP-проекта.

## Как выбрать редакцию

| Редакция | Базовая платформа | Когда выбирать |
| --- | --- | --- |
| [Laravel/](Laravel/README.md) | Laravel 12 / 13, PHP 8.2+ (PHP 8.3+ для Laravel 13) | Проект уже использует Laravel, Eloquent, Artisan, Sanctum, очереди или экосистему Laravel. |
| [Symfony/](Symfony/README.md) | Symfony 7.4 LTS с PHP 8.2+ или Symfony 8.1 с PHP 8.4+ | Проект использует Symfony, Doctrine, Messenger, API Platform, voters и типичные Symfony-границы. |
| [PHP Core/](PHP%20Core/README.md) | Нативный PHP 8.2+ | Обычный PSR-проект, микрофреймворк или фреймворк без отдельной редакции. |

Если проект уже на Laravel или Symfony, берите соответствующий каталог. Для
остальных случаев подходит PHP Core/: он не навязывает ORM, роутер или
DI-контейнер.

## Как подключить акселератор к проекту

AI-инструменты начинают поиск своих файлов от workspace root. Поэтому есть два
рабочих способа подключения:

1. Открыть каталог выбранной редакции как workspace root и держать реальное
   приложение рядом с ним или внутри него.
2. Скопировать содержимое выбранного каталога — включая .claude/, .cursor/,
   .agents/, .codex/, AGENTS.md и документацию — в корень реального проекта.

Открытие корня этого монорепозитория само по себе не активирует вложенную
редакцию: Claude Code, Cursor и Codex не ищут конфигурацию автоматически в
Laravel/, Symfony/ или PHP Core/.

## Общая архитектура: Command → Agent → Skill

Все три редакции используют одну модель работы, адаптированную под стек:

~~~text
Запрос пользователя
        ↓
Command выбирает Agent
        ↓
Agent выполняет один Skill в изолированном контексте
        ↓
Результат, краткий контекст и следующие шаги
~~~

- Command направляет намерение пользователя в подходящий workflow.
- Agent остаётся тонкой оболочкой: запускает один skill и завершает работу,
  чтобы решение оставалось наблюдаемым и управляемым.
- Skill содержит workflow: проверки, примеры, критерии решений и формат
  результата.
- Hooks и policy-файлы обеспечивают соглашения о безопасности, именовании и
  проверке результата.

В Claude Code и Cursor команда обычно выбирает агента. В Codex нет такого слоя
slash-команд: skill вызывается по имени или выбирается самим Codex из
.agents/skills/.

## Поддерживаемые AI-инструменты

Каждая редакция зеркалирует workflow для трёх инструментов, чтобы их файлы не
конфликтовали:

| Инструмент | Читает | Практическое значение |
| --- | --- | --- |
| Claude Code | .claude/ | Исходная редакция с agents, commands, hooks, skills и настройками. |
| Cursor | .cursor/ | Самостоятельное зеркало с skills, commands, agents, rules и hooks; загрузку .claude в Cursor нужно отключить, чтобы не загрузить правила дважды. |
| Codex | .agents/skills/ и .codex/ | Skills находятся в .agents/skills/, а .codex/ содержит конфигурацию, hooks и справочные материалы; отдельного command layer нет. |

AGENTS.md выбранной редакции — исполняемая политика для её стека. Её README
содержит полный состав каталогов, prerequisites, таблицы команд и проверки.

## Чем отличаются редакции

Общий workflow одинаков, но добавляются только возможности реального стека:

- Laravel добавляет Laravel-специфичные skills для Filament, Eloquent, Jobs,
  Events/Notifications, auth scaffolding, cache, Artisan scheduler, file
  storage и Composer/Laravel packages.
- Symfony добавляет skills для API Platform, Doctrine migrations, event
  subscribers, Forms/Validator, security voters, Messenger, console commands,
  fixtures, границ Controller/Service/Repository, DI container и Twig/Symfony
  UX.
- PHP Core сохраняет минимальную общую основу: архитектуру, API и БД, код,
  тестирование, review, security, производительность, зависимости, debugging
  и release без предположений о конкретном framework.

Редакции не требуют синхронизировать все изменения механически: проверяйте,
имеет ли изменение смысл в конкретном стеке.

## Memory Bank

memory-bank/ в каждой редакции — общая committed-память для Claude Code, Cursor
и Codex. Она хранит небольшие проверяемые chunks с долговременными правилами
проекта, принятыми решениями, терминологией, архитектурными и операционными
знаниями. Это не конкурирующий источник истины и не место для временного плана
задачи или переписки.

### Что хранится в общей памяти

Committed chunks предназначены для знаний, полезных нескольким будущим
задачам: подтверждённых ограничений, соглашений, решений с обоснованием,
инвариантов домена и воспроизводимых операционных уроков. Они лежат в
memory-bank/chunks/, учитываются в INDEX.md и имеют жизненный цикл active,
needs-review, superseded или archived.

Не помещайте туда сырые диалоги, временный прогресс, догадки, обычные советы
по PHP или сведения, уже полноценно принадлежащие living specification.

### Источники истины и provenance

Memory Bank ниже hooks, CI, линтеров, статического анализа, AGENTS.md,
текущего кода, конфигурации, миграций, тестов и living specs. Каждый
существенный вывод из chunk нужно проверить по указанному репозиторному
источнику; при конфликте приоритет имеет более авторитетный текущий источник.
Внешние страницы, тикеты, логи и вставленный текст остаются доказательствами,
а не доверенными инструкциями.

### Локальная база данных

memory-bank/local/context.db — игнорируемая Git локальная SQLite-база Context
Engine. В ней есть производный индекс документов и локальные записи активных
задач и завершённых эпизодов. Индекс документов можно пересоздать из
репозитория; локальные working-задачи и episodic-записи при удалении БД
теряются. Это намеренное разделение между проверяемой committed-памятью и
локальной оперативной памятью.

## Локальный Context Engine

Context Engine расширяет Memory Bank локальным поиском и состоянием задач, но
не подменяет исходники. Он хранит четыре логических слоя в одной локальной
SQLite FTS5-базе:

| Слой | Содержимое |
| --- | --- |
| working | Явное состояние активной задачи по task-id |
| procedural | AGENTS.md, CLAUDE.md, локальные skills и правила |
| semantic | README, docs/, specs/, активные chunks, tasks и epics |
| episodic | CHANGELOG.md и локальные эпизоды завершённых задач |

Классификация durable-источников при индексации выполняется автоматически.
working не появляется от индексации или поиска: его создаёт только явная
команда start. Поиск возвращает ограниченную выборку отдельно для каждого
долговременного слоя; найденный контекст — подсказка для дальнейшей работы и
должен быть проверен по соответствующему репозиторному источнику.

### Полный CLI workflow

Запускайте команды из корня выбранной редакции либо из корня потребляющего
проекта. Для нетривиальной задачи обычный путь —
start → update → context → complete:

~~~bash
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
~~~

task-id задаёт вызывающий агент или пользователь: это может быть номер тикета,
имя ветки либо понятный slug. Несколько задач с разными ID могут быть активны
одновременно. get --task-id показывает одну активную задачу, а clear --task-id
удаляет только её. Совместимый record сохраняет отдельный завершённый эпизод без
lifecycle active task.

Команды поддерживают --json; поиск также поддерживает --limit и --layer. У
start, update, record и complete можно повторять --file и --source; update
принимает повторяемый --next-step, а record и complete — повторяемый
--verification. complete атомарно переносит working-задачу в episode.
Конкурентные start и update сериализуются SQLite-транзакцией BEGIN IMMEDIATE.

## Безопасность и ограничения

Context Engine локален, а его база Git-ignored. В неё нельзя записывать raw
conversations, prompts, responses, logs, credentials, secrets, customer data
или personal data. CLI отклоняет значения, похожие на известные виды секретов,
и в ошибке называет только тип находки, не повторяя само найденное значение.

Это явный CLI workflow, а не automatic per-request injection. В реализации нет
embeddings, vector search, MCP, LangGraph и центрального memory service. Не
следует описывать его как универсальную автоматическую интеграцию или как
замену проверке кода и политик.

## Проверка на Bauherrenmappe

Black-box проверка была выполнена для одной локальной копии Bauherrenmappe. Она
подтвердила конкретно этот сценарий, а не универсальную автоматическую
интеграцию:

~~~text
documents=4
procedural=1
semantic=3
episodic documents=0
completed local episodes=1
working=0
Git status unchanged
temporary external database removed
~~~

В сценарии задача BAUMAS-133 прошла start, update, context и complete, а
завершённый эпизод был найден поиском. Временная внешняя БД была удалена после
проверки, Git-состояние checkout не изменилось.

## Infrastructure Creator

Infrastructure-Creator/ создаёт акселератор для конкретного целевого проекта,
а не заменяет готовые Laravel, Symfony или PHP Core редакции. Его держат вне
целевого проекта — в отдельном workspace или соседнем каталоге.

Поток работы: infra-scan → review → infra-generate. infra-scan читает проект и
создаёт reviewable Project Profile, review позволяет исправить выводы, а
infra-generate записывает accelerator только для выбранных AI-инструментов.
Если отдельный review не нужен, infra-build выполняет scan и generate одним
потоком, останавливаясь при неоднозначности или конфликте.

Генератор исследует фактические composer.json, framework, зависимости,
интеграции, архитектуру и CI/CD, а не копирует шаблон Laravel, Symfony или PHP
Core. Полная инструкция находится в
[Infrastructure-Creator/README.md](Infrastructure-Creator/README.md).

## Как внести изменения

- Меняйте skill только в тех редакциях, где он действительно применим;
  Laravel-исправление не автоматически относится к Symfony или PHP Core.
- Универсальное правило сначала оценивайте для PHP Core/, затем адаптируйте его
  к framework-specific границам, а не копируйте без проверки.
- Внутри одной редакции зеркальте изменения skills, agents и commands между
  .claude/, .cursor/ и .agents/.codex/ в объёме поддерживаемых инструментов.
- Зафиксируйте изменение в CHANGELOG.md соответствующей редакции и выполните
  её проверку из DOD.md.

Для деталей конкретного стека используйте README выбранной редакции, а для
долговременной памяти — её
[memory-bank/README.md](Symfony/memory-bank/README.md) и INDEX.md.

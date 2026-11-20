# План: SkillCache в Harness — наблюдение, компиляция и тёплое исполнение навыков

> Статус: план для будущей реализации, не начат. Дата: 2026-11-20.
> Основание: статья SkillCache (arXiv:2611.10022v1, «Execution-Grounded
> Specialization for Token-Efficient Skill Use») и исследование кода
> PR #107 (`feat/pr106-improvements`, коммит `cd826952`).
> Область: новый crate `runtime-rust/crates/infra-skillcache`, точечные
> правки `infra-host`, `infra-cli`, `infra-harness`, `infra-ui`, `harness/web`,
> `infra-eval`. Издания (Laravel/Symfony/PHP Core/WordPress) и
> Infrastructure-Creator не затрагиваются.
> Принятие документа не означает, что описанные функции существуют. Любая
> эвристика выбора, ранжирования или автоматической перекомпиляции
> включается по умолчанию только после собственного запечатанного измерения
> ([правило стенда](../MEMORY-EVAL.md)).

## 0. Кратко

SkillCache превращает повторяющееся использование навыка из «модель каждый
раз читает `SKILL.md` и заново выстраивает оркестрацию» в «короткий
детерминированный план, который исполняет локальный executor, возвращая
управление модели только там, где нужно рассуждение». Три фазы:

1. **Observe (cold).** Обычная сессия с неизменёнными навыками; Harness
   записывает наблюдаемую трассу `τ_obs` (вызовы инструментов, аргументы,
   результаты, ошибки, сообщения агента) без рассуждений провайдера и с
   редакцией секретов.
2. **Compile (изолированно).** Отдельный агент-компилятор без доступа к
   файлам задачи превращает `(q, S, τ_obs)` в параметризованную капсулу
   `z = (μ, B, P, π, V)` плюс квалификационные примеры `E`. Капсулу проверяют
   три хостовых гейта без вызовов модели; до 3 гибридных попыток, затем
   текстовый Markdown-фолбэк, затем исходный навык.
3. **Execute (warm).** На попадании промпт содержит только `μ`, заранее
   известные привязки `B`, начальные NL-шаги `π` и протокол `skillcache-run`.
   Программы исполняет `skillcache-run` через `skillcache-tool`; сбой
   программы деоптимизирует **только** этот шаг (`program_failed`), остальной
   план остаётся скомпилированным.

В Harness PR #107 всё это ложится на существующие механизмы (сессии,
`LaunchAdapter`, `infra_host::sandbox`, `infra_host::capture`, SQLite
`sessions.sqlite3`, SSE `StreamHub`, Wasm UI, `infra-eval`), но требует
нового нижнего crate, явного opt-in для хранения трассы и нескольких
исправлений в хосте (сеть в песочнице, multicall-диспетчеризация,
фильтр секретов).

## 1. Проблема, статья и prior art PR #107

### 1.1 Что фиксирует статья

| Элемент | Контракт |
| --- | --- |
| `τ_obs` | `((a_1,o_1),…,(a_T,o_T))`: вызовы инструментов, аргументы, результаты, ошибки, сообщения агента; без reasoning провайдера; секреты отредактированы; верификатор/оракул в cold-прогоне невидим. |
| Правила компиляции | path pruning, binding lifting, region fusion, grounded postconditions. |
| `μ` | `name` + `description`: то, по чему капсулу выбирают и показывают модели. |
| `B` | Привязки (bindings), известные до передачи управления (pre-handover). Содержимое файлов никогда не является привязкой. |
| `P` | Программы. `inputs`, `outputs` (только значения, которые читают **последующие** программы; никогда не повторяют `inputs`), `check`. Ответы идут в `result`, а не в `outputs`. |
| `π` | Упорядоченные шаги (`steps`): программные шаги и остаточные NL-шаги с указателями на исходный навык. |
| `V` | Верификация: выполняется **после** `π`; смесь read-only программ проверки и NL-шагов инспекции, выведенных строго из постусловий запроса/навыка или самопроверки, наблюдавшейся в `τ_obs`; никогда не выдумывается и не сравнивается с ответом cold-прогона. |
| `E` | `examples`: наблюдённые значения; используются **только** в Gate 3 и вырезаются при монтировании перед warm-исполнением. |
| Gate 1 | Структура и dataflow DAG. |
| Gate 2 | Свежая песочница: `check` каждой программы. |
| Gate 3 | Свежая песочница задачи: сквозной replay через `skillcache-run` c `E` для привязок, которые не производят программы; при сбое программы — диагностика, подстановка наблюдённого выхода из `E`, продолжение для сбора последующих сбоев. |
| Повторы | До 3 гибридных попыток → текстовый Markdown-фолбэк → исходный навык. |
| `skillcache-run` | Короткоживущий foreground CLI; файл состояния (`position`, `awaiting`, `bindings`, `failed`, `completed`) сохраняется **до** запуска каждой программы; ответы `needs_bindings` / `your_steps` / `complete`; подряд идущие программы исполняются без возврата к модели. |
| `skillcache-tool` | `SKILLCACHE_INPUT`, `SKILLCACHE_WORKSPACE`, `SKILLCACHE_PACKAGE`, `SKILLCACHE_ASSETS` (read-only); удаление переменных `*KEY*`/`*TOKEN*`/`*PASSWORD*`; stdout — один JSON `{"status":"complete","result":{...}}`. |
| Деоптимизация | После 2 ненулевых выходов, таймаута или отсутствия объявленного выхода: `{step_index, status:"program_failed", instruction, diagnostics, program_source_path?}`; `program_source_path` только для шагов `π` (для `V` — без пути). Модель выполняет шаг обычными инструментами и исходными навыками, не чинит и не перезапускает программу; остальной план остаётся скомпилированным. |
| Раздел 8 (production) | Выбор капсулы по `μ` при растущем кэше, инвалидирование при изменении навыка/окружения, согласование с правами и безопасностью, учёт амортизации и оценка. |

### 1.2 Что уже есть в PR #107 (расширяем, не дублируем)

| Существующее | Ссылка | Отношение к плану |
| --- | --- | --- |
| Жизненный цикл сессии | `Sessions::create_options` `sessions.rs:589` (ключи :590-614), `send` :2260 (ключи :2275-2288), `execute_job` :1578, `run_job` :1740 | Добавляется ключ `skillcache`; решение hit/miss — между `commands()?.route` (:2000) и `lifecycle.begin_with_options` (:2019). |
| Сборка промпта | `prompt::build` `prompt.rs` (native short-circuit :105-125, `route.text` :237, `skill_request` :238-243), `PromptLedger` | Новая категория ledger `skillcache` вместо `skill_request` на попадании. |
| Маршрутизация навыков | `CommandCatalog::route` `commands.rs:325`, Claude `/cmd` → `mode="native"` :374-385, `request()` :770-786 | Warm только для явно маршрутизированных навыков; native-маршрут — только observe. |
| Точка инъекции в запуск | `ProjectLaunchAdapter` `accelerators.rs:417` (`adapt` :424), `EvidenceLaunchAdapter` :563 (`adapt` :567) | Warm: PATH/env/allowlist; компилятор: шаблон изолированного адаптера. |
| Поток событий провайдера | `provider::run_monitored_locked` `infra-host/src/provider.rs:407`, `Reading::read` :565, trait `RunEvents` :115, `activity::normalize` `infra-host/src/activity.rs:847` | Единственный слот `RunEvents` занят `RunBinding` (`sessions.rs:2126`) — нужен tee для `TraceRecorder`. |
| Учёт стоимости | `observe_binding` `lifecycle.rs:756`, `finish_accounting` :1026, `cost_observation.rs` (метки фаз только `main`/`recovery` :132-133, :627-629), `ContextTracker` `context_usage.rs` | Источник токенов warm/cold; компиляция учитывается отдельно. |
| Результаты запусков | `Results::start_launch` `results.rs:409` (`kind` :428-436), `history` :481-494 | `launches.id` = generation; компиляцию туда **не** пишем. |
| Песочницы ОС | `infra_host::sandbox::Spec` `infra-host/src/sandbox.rs:12-21`, `creator` :188, `discovery` :191, `trusted_helper` :142 | Добавить `network`, режим `gate`; переиспользовать boundary-probe. |
| Захват процессов | `infra_host::capture::run` `infra-host/src/capture.rs:62` (полная замена окружения, лимиты, deadline) | Основа `skillcache-tool` и Gate 2/3. |
| Хранилище | `SessionStore` `store.rs:175` (`sessions.sqlite3` открывается в :271), ALTER-колонки :295, восстановление после рестарта :329-344, `inbox::initialize/recover` | Таблицы `skillcache_*` в том же файле по образцу `inbox.rs`. |
| SSE | `StreamFrame.event: &'static str` `store.rs:19`, кольцо 1024 `store.rs:11`, `matches_sse_filter` `http.rs:2019-2045`, `DEFAULT_SSE_EVENTS` `infra-ui/src/api.rs:143-157` | Один новый статический кадр `skillcache_capsule`; прогресс — через `session_event`. |
| HTTP | `Route` `http.rs:289-296`, `ROUTES` :297, DELETE поддержан (:838, :3007), `POST /api/projects/{project_id}/preflight` :1207 | Маршруты вложены в `/api/projects/{project_id}/…` и `/api/sessions/{session_id}/…`. |
| Структурированный ответ агента | `infra-system/src/native.rs:238` (`--output-schema` Codex), :254 (`--json-schema` Claude), :391 (`structured_output`) | Компилятор возвращает капсулу структурированным ответом; файлы пишет хост. |
| Оценка | `outcome::validate_design` `infra-eval/src/outcome.rs:506`, `seal` :627, `run_complete` :826, `summarize_complete` :1082, `known_usage` :1294, `isolation::prepare` `isolation.rs:55`, `ScriptedProvider` `bench.rs:202`, `rng::bootstrap` `rng.rs:185` | Новый протокол `skillcache-eval-protocol-v1` поверх тех же механизмов. |
| UI | `State` `infra-ui/src/model.rs:181`, `run/outcomes.rs` `TABS` :146, `session/navigation.rs:578`, `eval_viewer.rs:251` | Вкладка запуска `skillcache`, вид `skill-capsules`, конфигурации eval viewer. |
| A/B-стенд сессий | [HARNESS-SESSION-AB](../HARNESS-SESSION-AB.md) | Образец запечатанного плана и учёта неизвестных как `null`. |

### 1.3 Коллизии и ограничения, найденные в коде

1. **Имя «capsule» занято**: `infra_core::capsule::Capsule` (`infra-core/src/capsule.rs:43`),
   Brain-капсула памяти (`PromptLedger::capsule`, `CONTEXT_CAPSULE_DELIVERED`,
   `lifecycle.rs` before_dispatch), капсула делегирования (`blackboard.rs`). Решение:
   тип `SkillCapsule`, таблицы и ключи `skillcache_*`, вид UI `skill-capsules`.
2. **Контракт приватности**: `run_activity.rs` и `cost_observation.rs` обещают не
   хранить вывод инструментов; `activity.rs` режет детали до `DETAIL_LIMIT = 300`
   (`infra-host/src/activity.rs:5`), результаты инструментов не сохраняются.
   Хранение `τ_obs` — изменение контракта, только явный opt-in.
3. **Native Claude `/cmd`** (`commands.rs:374-385`, `prompt.rs:105-125`) обходит все
   инструкции Harness: warm-инъекция невозможна, допустимо только наблюдение.
4. **Единственный слот `RunEvents`** (`sessions.rs:2126`) занят `RunBinding`.
5. **`history()` перечисляет все `launches` сессии** (`results.rs:481-494`):
   строка компиляции стала бы фантомным ходом.
6. **Guardian процесса**: `guardian()` возвращает `current_exe()`
   (`infra-host/src/process.rs:1756-1760`) и запускает `<guardian> internal-guard`
   (:934), причём `capture::run` сам идёт через `run_captured_until` →
   `launch_guardian_options` (`process.rs:555, :906-933`), так что **каждая**
   программа, запускаемая `skillcache-tool`, проходит через `guardian()`. Для
   hardlink-а `skillcache-run`/`skillcache-tool` это превращается в
   `<имя-линка> internal-guard …`; `infra-cli/src/main.rs` сейчас не смотрит на
   `argv[0]`.
7. **Codex Manual**: `thread/start` с `"sandbox": "read-only"`,
   `"approvalPolicy": "on-request"` (`infra-host/src/conversation.rs:1356-1360`) —
   каждая запись `plan-state.json` станет запросом подтверждения.
8. **Песочница без выключения сети**: `Spec` не имеет поля сети; bwrap без
   `--unshare-net`, Windows TOML `"network":{"enabled":true}` (`sandbox.rs:242`).
9. **Нет фильтра секретных переменных окружения** для дочерних процессов
   (есть только редакция текста: `run_activity::redact_command` `run_activity.rs:93`,
   `agent_activity::redact` `agent_activity.rs:412`).
10. **Парность навыков в индексе** (`infra-core/src/index.rs:1353-1364`,
    `walk_markdown_metadata`) хэширует только `.md`: для инвалидирования капсулы
    нужен собственный отпечаток всех файлов навыка.
11. **Направление зависимостей**: `infra-tooling` зависит от `infra-host` под
    `cfg(windows)`; `infra-harness` не зависит от `infra-eval`/`infra-skill-tools`;
    `infra-ui` (Wasm, edition 2021) не зависит ни от одного crate рабочего
    пространства.

## 2. Зафиксированные решения

| Развилка | Решение |
| --- | --- |
| Где живёт ядро | Новый crate `infra-skillcache` (зависимости: `infra-core`, `infra-host`, `serde`, `serde_json`, `sha2`, `regex`; без `tokio`). Ниже `infra-harness`, `infra-eval`, `infra-cli`; циклов нет. `infra-system` и `infra-skill-tools` не используются для исполнения: первый оркестрирует многорепозиторные AI-диспетчеризации, второй — авторские утилиты навыков. |
| Где Harness-обвязка | `infra-harness/src/skillcache/` (трасса, компиляция, гейты 2/3, БД, HTTP-домен). |
| Где оценка | `infra-eval/src/skillcache.rs` (зависит от `infra-skillcache`). |
| UI | `infra-ui` остаётся без зависимостей от crate-ов; DTO читаются как `serde_json::Value`, терпимы к отсутствующим/новым полям. |
| БД | Таблицы `skillcache_*` в `sessions.sqlite3` (`store.rs:271`) с `skillcache::initialize`/`recover` по образцу `inbox.rs`. Отдельного файла БД нет: все домены Harness уже живут в одном файле, `graph_ir.sqlite3` — перестраиваемый индекс и не прецедент. Ничего не пишется в `memory-bank/local/context.db`, `memory-bank/` или `project-brain/`. |
| Имена | Rust: `SkillCapsule`; SQL: `skillcache_capsules`, `skillcache_capsules_fts`, `skillcache_gates`, `skillcache_runs`, `skillcache_traces`, `skillcache_compiles`; ключ сессии и категория ledger: `skillcache`; SSE: `skillcache_capsule`; UI: вид `skill-capsules` («Skill capsules»), вкладка запуска `skillcache`. |
| Режим сессии | `skillcache: {mode: "off"|"observe"|"auto"|"warm_only", compiler: {provider, model}}`. **По умолчанию `off`.** `observe` — только запись трассы; `auto` — warm на попадании, иначе cold+observe и компиляция по явному действию; `warm_only` — warm на попадании, иначе обычный cold без записи. Автоматическая компиляция после каждого cold-прогона — отдельный переключатель `compile_after_observe`, выключен до измерения. `workflow` остаётся `native`, чтобы не ломать inbox (`inbox.rs` eligibility). |
| Компилятор | Изолированный запуск без файлов задачи; ответ — структурированный (`--json-schema`, образец `infra-system/src/native.rs:254`, `:391`), режим plan, shell отключён (`--disallowedTools Bash,Agent,Task`); файлы капсулы пишет хост. Не Write/Edit в песочнице. **v1: компилятор только Claude**: только у него инструментальный запрет shell гарантирует «никакого исполнения программ при компиляции»; Codex read-only и Cursor plan сохраняют shell для чтения — это отклонение от статьи, поэтому они вне v1 (исполнитель cold/warm при этом может быть другим провайдером). |
| Сеть в гейтах | Gate 2 — без сети. Gate 3 — та же сетевая политика, которую получит warm-исполнение: капсула несёт `capabilities.network: "none"|"required"`, выведенное из `τ_obs`; по умолчанию `none`. |
| Хранение артефактов | `<state>/skillcache/capsules/<project_key>/<capsule_id>/<rev>/warm/` (без `E`), `<state>/skillcache/qualification/<capsule_id>/<rev>/examples.json` (только Gate 3), трасса — строка `skillcache_traces`. Экспорт в репозиторий — вне v1. |
| Учёт компиляции | Отдельная таблица `skillcache_compiles` с той же формой `usage`/`receipt`, что у `launches`; амортизация считается в домене SkillCache. |
| Warm-маршрут | v1: ровно один явно маршрутизированный навык (`route.requests`; Codex может назвать до 5 `$skill`, `commands.rs:415-445` — тогда промах), сессия `mode="edit"`, провайдер Claude или Codex (не native Claude `/cmd`, не Codex Manual). Cursor в v1 только observe: запись вне рабочей копии в его песочнице и проброс окружения не проверены (адаптер на них не опирается, см. [AI-SYSTEM-ORCHESTRATION.md](../AI-SYSTEM-ORCHESTRATION.md)). Выбор по `μ` (FTS5) — эксперимент за флагом, пока не измерен. |
| Перекомпиляция по деоптимизациям | Только показ и ручное действие в v1; пороги — эвристика, выключены до измерения. Статус капсулы `needs_recompile` (не «deoptimized»: в статье деоптимизация — свойство шага в прогоне). |
| Хранение трассы | Явный opt-in на сессию: только `mode ∈ {observe, auto}` (не `off` и не `warm_only`), документируется в [harness/README.md](../../harness/README.md) и [CHANGELOG.md](../../CHANGELOG.md); файлы/строки в приватном state (0700/0600), удаляются вместе с капсулой. |
| Инструкции компилятора | Версионированный ресурс `infra-skillcache/resources/compile/` (`include_str!`), не навык изданий: иначе сработали бы зеркала, инвентари, budget, changelog изданий и установка клиентам. |

## 3. Контракты

### 3.1 Схема капсулы (`capsule.json`, `schema_version: 1`)

```json
{
  "schema_version": 1,
  "form": "hybrid",
  "mu": {"name": "pdf-invoice-totals", "description": "Extract totals from invoice PDFs into a CSV summary"},
  "source": {"skills": [{"name": "pdf", "path": ".claude/skills/pdf/SKILL.md", "tree_sha256": "…"}]},
  "capabilities": {"network": "none", "runtimes": ["python3"]},
  "bindings": [
    {"name": "input_dir", "type": "path", "origin": "request"},
    {"name": "currency", "type": "string", "origin": "request"}
  ],
  "programs": [
    {
      "id": "extract",
      "entry": ["python3", "programs/extract/main.py"],
      "inputs": ["input_dir", "quarter_files"],
      "outputs": ["rows_file"],
      "check": ["python3", "programs/extract/main.py", "--check"],
      "timeout_seconds": 120,
      "read_only": false
    },
    {
      "id": "summarize",
      "entry": ["python3", "programs/summarize/main.py"],
      "inputs": ["rows_file", "currency"],
      "outputs": ["summary_file"],
      "check": ["python3", "programs/summarize/main.py", "--check"],
      "timeout_seconds": 60,
      "read_only": false
    },
    {
      "id": "verify_csv",
      "entry": ["python3", "programs/verify_csv/main.py"],
      "inputs": ["summary_file", "quarter_files"],
      "outputs": [],
      "check": ["python3", "programs/verify_csv/main.py", "--check"],
      "timeout_seconds": 30,
      "read_only": true
    }
  ],
  "steps": [
    {"kind": "instruction", "text": "Confirm which invoices belong to the requested quarter.", "source": {"path": ".claude/skills/pdf/SKILL.md", "section": "Selecting files"}, "produces": ["quarter_files"]},
    {"kind": "program", "program": "extract"},
    {"kind": "program", "program": "summarize"}
  ],
  "verification": [
    {"kind": "program", "program": "verify_csv"},
    {"kind": "inspection", "text": "Open the CSV and confirm one row per invoice."}
  ]
}
```

`examples.json` хранится отдельно (`{"bindings": {...}, "steps": {"<index>": {"result": {...}}}}`)
и в warm-пакет не попадает. Текстовая форма (`form: "textual"`) — `capsule.md` с тем же
`mu` и `source`, без `programs`.

Выход программы `summarize` (stdout, контракт 3.3): объявленный `output` и ответ лежат в
`result`; связываются только объявленные имена, остальные ключи — ответный материал для
модели:

```json
{"status":"complete","result":{"summary_file":"out/q3-summary.csv","total":"18240.00","invoices":12}}
```

Инварианты (проверяет Gate 1, `infra-skillcache/src/gate1.rs`):

- Строгий JSON (`infra_core::strict_json`), неизвестные поля отклоняются, лимиты размера.
- Уникальные `id`; каждый `steps[].program` и `verification[].program` существует.
- Dataflow DAG: каждый `input` программы доступен из `bindings`, `outputs` более раннего
  программного шага или `produces` более раннего NL-шага; циклов и висячих ссылок нет.
- `outputs` не пересекаются с `inputs` той же программы и каждое значение читается хотя
  бы одним последующим шагом; ответы — в `result`, не в `outputs`.
- Программы в `verification` помечены `read_only: true` и стоят после всех `steps`.
- Привязки — скаляры/пути ограниченного размера (≤ 4 KiB); значение, совпадающее с
  содержимым файла из `τ_obs`, отклоняется (`file_content_binding`).
- `entry`/`check` — argv без shell-строк, пути внутри пакета, без `..`, ссылок и
  абсолютных путей; на Windows — не `.cmd`/`.bat` (как `provider::command_argv`).
- `E` не встречается в `capsule.json`; секреты (`infra_core::domain::privacy`) отсутствуют и в
  `capsule.json`, и в `examples.json`.

### 3.2 `skillcache-run` и файл состояния

Команды: `skillcache-run run ['<JSON bindings>']`, `skillcache-run continue ['<JSON>']`,
`skillcache-run status`. Окружение задаёт Harness: `SKILLCACHE_PACKAGE`,
`SKILLCACHE_WORKSPACE`, `SKILLCACHE_STATE_FILE` (внутренняя переменная executor-а; в
программы не передаётся).

Файл состояния (атомарная замена + fsync через `infra_core::fs`, до запуска **каждой**
программы). Пять полей верхнего уровня статьи сохраняются; служебные данные живут внутри
`awaiting`:

```json
{"schema_version":1,"capsule_sha256":"…","generation":"…","position":3,
 "awaiting":{"kind":"running","step_index":3,"attempt":1},
 "bindings":{"input_dir":"invoices/","quarter_files":["q3-01.pdf","q3-02.pdf"]},
 "failed":[{"step_index":2,"reason":"nonzero_exit"}],
 "completed":[0,1]}
```

Формы `awaiting`:

| `kind` | Поля | Смысл |
| --- | --- | --- |
| `running` | `step_index`, `attempt` (1 или 2) | записывается до запуска программы; переживает аварию |
| `bindings` | `step_index`, `names` | ответ `needs_bindings` |
| `steps` | `step_indexes`, `produces` | ответ `your_steps` |
| `outputs` | `step_index`, `names` | после `program_failed`: объявленные выходы упавшего шага, которые читают последующие шаги |
| `none` | — | план завершён (`complete`) |

Ответ (stdout, один JSON):

| `status` | Когда | Содержимое |
| --- | --- | --- |
| `needs_bindings` | следующий программный шаг требует значения, которого нет в `bindings` | `names`, описания из `bindings[]` |
| `your_steps` | следующий шаг — NL (`π` instruction или `V` inspection) | список подряд идущих NL-шагов с `step_index`, `instruction`, `source`, ожидаемые `produces` |
| `program_failed` | 2 ненулевых выхода, таймаут, отсутствие объявленного выхода или авария во 2-й попытке | `{step_index, status, instruction, diagnostics, program_source_path?, required_outputs}`; путь — только для шагов `π` |
| `complete` | все `steps` и `verification` пройдены | агрегированный `result` |

Правила:

- Подряд идущие программы выполняются без возврата модели.
- Ненулевой выход — один повтор (`attempt` 1 → 2); таймаут и отсутствие объявленного
  `output` — сразу сбой.
- Шаг из `failed` больше не исполняется. После `program_failed` состояние переходит в
  `awaiting:{kind:"outputs"}`: `continue '<JSON>'` обязан содержать все объявленные выходы
  упавшего шага, которые читают последующие шаги (модель получает их обычными
  инструментами и исходными навыками); без них `continue` повторяет `program_failed` с
  `required_outputs`. Для шага `V` без выходов `continue` просто идёт дальше.
  `skillcache-tool` отказывает программе, помеченной `failed` в файле состояния
  (`refused_failed_program`).
- После аварии процесса или таймаута команды `continue` читает
  `awaiting:{kind:"running",step_index,attempt}`: при `attempt = 1` шаг запускается ещё раз
  как попытка 2; при `attempt = 2` шаг получает `program_failed` с причиной `interrupted`.
  Побочные эффекты частичного запуска в рабочей копии не откатываются; это указывается в
  `diagnostics`.
- Каждое событие дописывается в `run-log.jsonl` рядом с файлом состояния (append + fsync,
  монотонный `seq`) — мост к Harness (3.6).

### 3.3 `skillcache-tool`

`skillcache-tool <program_id> --input '<JSON>'` или `--check`. Запуск через
`infra_host::capture::run` (`complete_environment`, лимит вывода, deadline):

- Окружение программы: наследуется всё окружение `skillcache-tool`, кроме переменных, имя
  которых без учёта регистра содержит `KEY`, `TOKEN` или `PASSWORD`; гарантированно
  присутствуют `PATH`, `HOME`/`USERPROFILE`, `SYSTEMROOT`/`COMSPEC`/`PATHEXT`,
  `TMPDIR`/`TMP`/`TEMP` (если были у родителя) и добавляются `SKILLCACHE_INPUT`,
  `SKILLCACHE_WORKSPACE`, `SKILLCACHE_PACKAGE`, `SKILLCACHE_ASSETS`; `SKILLCACHE_STATE_FILE`
  удаляется. Так как `capture::run` заменяет окружение целиком (`complete_environment`),
  карта строится явно. Фильтр применяется только к программам, никогда к процессу
  провайдера.
- stdout — ровно один JSON-объект `{"status":"complete","result":{...}}`; `result` обязан
  содержать каждый объявленный `output`. Иначе — ненулевой код и ограниченная
  отредактированная диагностика stderr.
- `SKILLCACHE_ASSETS` и `SKILLCACHE_PACKAGE` read-only: в песочнице — `Spec.readable`
  (bwrap `--ro-bind`, SBPL deny write, Windows `read`); в хостовом warm-режиме — файлы
  0444/каталоги 0555 и проверка SHA-256 пакета до и после запуска.

### 3.4 Гейты, повторы, фолбэк

| Гейт | Где | Без модели | Проходит, если |
| --- | --- | --- | --- |
| Gate 1 | процесс Harness, `gate1.rs` | да | все инварианты 3.1 |
| Gate 2 | свежая `sandbox::gate`, без сети, без файлов задачи | да | `check` каждой программы завершился по контракту 3.3 |
| Gate 3 | свежая копия задачи (`<state>/skillcache/replay/<run_id>/workspace`), сеть по `capabilities.network` | да | `skillcache-run --replay` прошёл все `steps` и `verification` без сбоев; `E` отвечает на `needs_bindings` и NL-шаги; при сбое программы выход подставляется из `E` и replay продолжается, собирая все последующие сбои. Так как `sandbox::gate` даёт минимальный корень (как `discovery`), Gate 3 намеренно строже хостового warm-запуска; отказ песочницы помечается кодом `sandbox_denied` |

Цикл: попытка 1 → (сбой) пакет диагностик всех гейтов → попытка 2 → попытка 3 → текстовый
фолбэк (`form: "textual"`; проверяется облегчённый Gate 1T — **наше решение, вне статьи**:
структура Markdown, лимит размера, отсутствие `E` и секретов; Gate 2/3 неприменимы) →
сбой фолбэка → `compile_failed`, используется исходный навык. Каждый раунд пишет
`skillcache_gates(capsule_id, round, gate, passed, report_json)`.

### 3.5 Песочницы фаз

| Фаза | Видит | Не видит | Сеть | Инструменты |
| --- | --- | --- | --- | --- |
| Cold (observe) | обычная сессия: запрос, навыки, рабочая копия | верификатор/оракул (в eval) | как сессия | как сессия |
| Compile | `request.json`, `tau_obs.json` (отредактировано), копии файлов `source_skills/`, `compile/` | проект, git common dir, `E` других капсул, временный sentinel (проверяется `boundary_request`) | только API провайдера | Claude `--disallowedTools Bash,Agent,Task`, mode=plan; ответ структурированный; Codex/Cursor в роли компилятора вне v1 |
| Gate 2 | warm-пакет (RO), пустой scratch | файлы задачи | нет | только `skillcache-tool --check` |
| Gate 3 | копия задачи (RW), raw-пакет с `E` (RO), state (RW) | реальная рабочая копия | по `capabilities.network` | только `skillcache-run`/`skillcache-tool` |
| Warm | рабочая копия, warm-пакет без `E`, `bin/` с инструментами, state каталога сессии (RW) | `qualification/` | как сессия | обычные + `skillcache-run`/`skillcache-tool` |

Компиляция и Gate 2/3 на Linux требуют bwrap, на macOS — `sandbox-exec`, на Windows —
elevated Codex (`sandbox.rs` creator/discovery); без них — отказ без понижения изоляции
(как Creator и AI discovery в [harness/README.md](../../harness/README.md)).

### 3.6 Warm-промпт, попадание и мост событий

- Решение hit/miss в `run_job` после `route` (`sessions.rs:2000`) и до
  `begin_with_options` (`:2019`): `mode ∈ {auto, warm_only}`; сессия `mode == "edit"`
  (в `plan` Codex read-only и не запишет файл состояния, а программы с `read_only: false`
  нарушают plan-режим); `route.requests` содержит **ровно один** явно маршрутизированный
  навык (Codex допускает до 5 `$skill`, `commands.rs:415-445` — больше одного в v1 даёт
  промах); провайдер Claude или Codex (не native Claude `/cmd`, не Codex Manual, не
  Cursor); найдена активная капсула с совпавшими `tree_sha256`, `env_fingerprint`,
  `schema_version`, `compile_rules_version`; и `capabilities.network != "required"` при
  провайдере, чья песочница запрещает сеть (например Codex `workspace-write` без сети —
  тогда промах).
- `prompt::build`:
  - На попадании **`form: "hybrid"`** вместо `route.skill_request()` (`prompt.rs:238-243`)
    — блок ledger `skillcache`: `μ` («name: description»), pre-handover `B`, начальные
    NL-шаги `π`, протокол `skillcache-run` и указание, что исходный навык доступен по
    пути на случай `program_failed`. Без `E`, без кода программ, без путей пакета.
  - На попадании **`form: "textual"`** вместо `route.skill_request()` — содержимое
    `capsule.md` в блоке ledger `skillcache` с указанием на исходный навык при пробелах;
    `skillcache-run` и `skillcache-tool` **не** используются (`bin/` не стадируется,
    `SKILLCACHE_*` не передаются), те же запреты на `E` и код программ.
  - На промахе — прежний `commands::request()`.
- Решение записывается в `settings.skillcache` при `start_launch` (`results.rs:409-424`) и
  событием сессии `{"kind":"skillcache","phase":"hit"|"miss",…}`.
- Мост: `skillcache-run` пишет `run-log.jsonl` (каждая строка содержит `(generation, seq)`).
  `RunBinding` при завершении tool-вызова, чей префикс команды в `targets.detail`
  (ограничено `DETAIL_LIMIT = 300`, `activity.rs:5`) начинается с `skillcache-run` (с
  возможным путевым префиксом), плюс финальный проход в `lifecycle.finish`
  (`lifecycle.rs:1097`) читает новые строки идемпотентно по `(generation, seq)` и
  публикует `session_event {"kind":"skillcache","phase":"needs_bindings"|"your_steps"|"complete"|"program_failed",…}`
  через `guarded_event`. Разбор результатов инструментов провайдера не нужен.

### 3.7 Трасса `τ_obs`

- `TraceRecorder: RunEvents` получает сырые события через tee-обёртку вокруг `RunBinding`
  (`sessions.rs:2126`) только при `mode ∈ {observe, auto}` и только на cold-ходе (warm-ход
  не записывается как `τ_obs`).
- Пары по провайдерам: Claude `tool_use` ↔ `tool_result` (`tool_use_id`, `is_error`);
  Codex `item.started`/`item.completed` (`command`, `aggregated_output`, `exit_code`);
  Cursor `tool_call` started/completed. Reasoning (`thinking`, `redacted_thinking`, codex
  reasoning, cursor thinking) отбрасывается; потоки помощников помечаются
  (`parent_tool_use_id`).
- Редакция: `agent_activity::redact` + `run_activity::redact_command` + точная замена
  значений секретных переменных окружения запуска + `strip_root`/unroot; 64 KiB на
  наблюдение с маркером усечения, общий лимит; счётчики редакций в строке трассы.
- Источники навыков `S`: `route.requests` + обнаружение в трассе (Claude `Skill`
  tool_use, чтение `*/SKILL.md`, Codex-команда, читающая `SKILL.md`).
- Компилируется только трасса запуска, завершившегося `completed` без ошибок провайдера.

### 3.8 Инвалидирование

- `tree_sha256` по **всем** обычным файлам каталога навыка (`SKILL.md`, `scripts/`,
  `references/`, `assets/`): быстрый stat `(mtime_ns, size)`, пересчёт SHA-256 при
  расхождении. Изменение → `status = "stale"`, warm не выполняется.
- `env_fingerprint = sha256({os, arch, engine_digest, версии runtimes из capabilities})`.
- Смена `schema_version` или версии инструкций компилятора → `stale`.
- Дрейф кода проекта ловит деоптимизация во время прогона; ручная переквалификация
  (Gate 1–3) доступна из UI.

### 3.9 Безопасность и приватность

- Трасса и капсулы — только приватный state Harness, вне Git проекта.
- Пакет warm монтируется read-only и не добавляется в `--add-dir` модели: его читает
  `skillcache-run`, не модель.
- Инструменты стадируются как hardlink/копия `infra-engine` (0500, без symlink, без
  `.cmd`/`.bat`); Claude `--settings permissions.allow` получает
  `Bash(skillcache-run:*)` и `Bash(skillcache-tool:*)`.
- Codex `workspace-write`: каталог состояния сессии добавляется через `--add-dir`.
- Bypass Permissions не меняет фильтр секретов `skillcache-tool`.

## 4. Пофайловый blueprint

### 4.1 Новый crate `runtime-rust/crates/infra-skillcache`

| Файл | Содержимое |
| --- | --- |
| `Cargo.toml` | `edition = "2024"`, `rust-version = "1.89"`; зависимости из 2. |
| `src/lib.rs` | Публичные модули и `execute_run`/`execute_tool` для CLI. |
| `src/capsule.rs` | `SkillCapsule`, `Binding`, `Program`, `Step`, `Verification`, `Examples`, `Form`, (де)сериализация со строгим разбором. |
| `src/gate1.rs` | Инварианты 3.1, коды нарушений, Gate 1T. |
| `src/package.rs` | Материализация `warm/` (без `E`) и `raw/`, права 0444/0555, дайджесты. |
| `src/state.rs` | Файл состояния, атомарная запись, восстановление. |
| `src/executor.rs` | Машина состояний `skillcache-run`, повторы, отказ от повторного запуска, `run-log.jsonl`. |
| `src/tool.rs` | `skillcache-tool`: окружение, контракт stdout, read-only проверки. |
| `src/env_filter.rs` | Фильтр `*KEY*`/`*TOKEN*`/`*PASSWORD*` (Unix/Windows, без учёта регистра). |
| `src/trace.rs` | Типы `τ_obs`, попарное сведение событий по провайдерам, лимиты, обнаружение навыков. |
| `src/fingerprint.rs` | `tree_sha256`, `env_fingerprint`. |
| `resources/compile/` | Инструкции компилятора (4 правила), JSON Schema ответа, шаблон текстовой формы. |
| `tests/` | Фикстуры капсул (валидные и по одной на каждое нарушение), executor end-to-end на фикстурных программах (Rust-бинарь `[[bin]] infra-skillcache-program-fixture`, без Python). |

### 4.2 `infra-host`

- `sandbox.rs`: `Spec.network: bool` (по умолчанию `true` для существующих вызовов); bwrap
  `--unshare-net`, SBPL `(deny network*)`, Windows `"network":{"enabled":false}`; новый
  `pub fn gate(spec, argv)` — минимальный корень как у `discovery`, без `/` (поэтому Gate 3
  строже хостового warm-запуска; отказ фиксируется кодом `sandbox_denied`).
- `process.rs`: тест, что guardian корректно работает при запуске через multicall-имя.
- Нет новых зависимостей; `infra-host` не зависит от `infra-tooling`.

### 4.3 `infra-cli`

- `main.rs`: multicall по стему `argv[0]` (`skillcache-run`, `skillcache-tool`, с `.exe`
  на Windows); если первый аргумент `internal-guard` или `internal`, диспетчеризация идёт
  обычным путём. Альтернатива — `infra_host::process::use_guardian(<путь infra-engine>)`
  до любого дочернего процесса; выбирается в M1 с тестом на обеих ОС.
- `lib.rs`: группа `skillcache` (`infra-engine skillcache run|continue|status|tool|gate1`)
  в `GROUPS` и диспетчере; добавить `"skillcache-ab"` в список eval-семейств с
  подкомандами (`lib.rs:877-881`); `arguments.rs` — новые флаги (`--input`, `--check`,
  `--replay`).

### 4.4 `infra-harness`

| Место | Изменение |
| --- | --- |
| `src/lib.rs` | `pub mod skillcache;` |
| `src/skillcache/mod.rs` | `SkillCacheDomain: Domain` (операции HTTP), фасад. |
| `src/skillcache/db.rs` | `initialize`/`recover` таблиц (4.6); вызов из `store.rs` рядом с `inbox::initialize` и `inbox::recover` (:328, :341). |
| `src/skillcache/trace.rs` | `TraceRecorder: RunEvents`, tee с `RunBinding`, запись `skillcache_traces`. |
| `src/skillcache/compile.rs` | `CompilerLaunchAdapter` по образцу `EvidenceLaunchAdapter` (`accelerators.rs:563`), `run_monitored_locked`, структурированный ответ, собственный `LaunchAccounting`, запись `skillcache_compiles`. |
| `src/skillcache/gates.rs` | Gate 2/3 через `infra_host::sandbox::gate` + `capture::run`; копия задачи по образцу `delivery.rs` (worktree) или `creator_files::validation_copy`. |
| `src/skillcache/queue.rs` | Ограниченная очередь компиляции/гейтов вне очереди сессий, с lease на git common dir (как `session_scheduler::Lease`) и восстановлением после рестарта. |
| `src/skillcache/warm.rs` | Решение hit/miss, стадирование `bin/`, env, `readable`, allowlist; ingest `run-log.jsonl`. |
| `sessions.rs` | Ключ `skillcache` в `create_options` (:590-614) и `send` (:2275-2288); hit/miss между :2000 и :2019; tee для `RunEvents` (:2126); фоновая постановка компиляции после успеха рядом с `spawn_omnigraph_publish` (:1708) только при `compile_after_observe`. `enqueue_check` (:1199) не используется. |
| `store.rs` | Колонка `skillcache TEXT` в ALTER-списке (:295). |
| `prompt.rs` | Категория `skillcache` (замена `skill_request` на попадании, :238-243). |
| `context_usage.rs` | Новый `kind` ledger `skillcache`. |
| `accelerators.rs` | `ProjectLaunchAdapter::adapt` (:424) принимает warm-дополнения (PATH, env, allowlist). |
| `results.rs` | `settings.skillcache` в `start_launch` (:409-424). |
| `lifecycle.rs` | Финальный ingest `run-log.jsonl` в `finish` (:1097). |
| `http.rs` | Маршруты 4.5; `skillcache_capsule` в исключениях фильтра по `session_id` (:2031-2037). |
| `services.rs` | Регистрация `SkillCacheDomain`; `boot["skillcache"]` (доступность песочниц, режим по умолчанию `off`). |
| `tests/support/browser_e2e_fixture.rs` | Фикстурные капсулы и последовательности событий для E2E. |

### 4.5 HTTP и SSE

| Метод и путь | Операция | `token` |
| --- | --- | --- |
| `GET /api/projects/{project_id}/skillcache/capsules` (query: `skill`, `status`, `form`) | `skillcache.list` | нет |
| `GET /api/projects/{project_id}/skillcache/capsules/{capsule_id}` | `skillcache.get` | нет |
| `POST /api/projects/{project_id}/skillcache/capsules/{capsule_id}/requalify` | `skillcache.requalify` | да |
| `POST /api/projects/{project_id}/skillcache/capsules/{capsule_id}/invalidate` | `skillcache.invalidate` | да |
| `DELETE /api/projects/{project_id}/skillcache/capsules/{capsule_id}` | `skillcache.delete` | да |
| `POST /api/sessions/{session_id}/skillcache/compile` (`{generation, compiler:{provider,model}}`) | `skillcache.compile` | да |
| `GET /api/sessions/{session_id}/skillcache` | `skillcache.session` | нет |

Предпросмотр попадания — поле ответа существующего `POST /api/projects/{project_id}/preflight`.
Компиляция «по ID навыка» не поддерживается: без наблюдённой генерации нет `τ_obs`.

SSE: (1) прогресс внутри сессии — существующий кадр `session_event` с
`{"kind":"skillcache","phase":…}`; (2) межсессионный статический кадр
`skillcache_capsule` с `state ∈ {queued, compiling, gate1, gate2, gate3, repair, qualified,
textual_fallback, failed, invalidated, requalified, deleted, stale}`, с `project_id`;
добавляется в `DEFAULT_SSE_EVENTS` (`infra-ui/src/api.rs:143-157`) и в исключения
`matches_sse_filter`.

### 4.6 Схема SQLite (`sessions.sqlite3`)

```sql
CREATE TABLE IF NOT EXISTS skillcache_capsules(
  capsule_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, skill_name TEXT NOT NULL,
  skill_path TEXT NOT NULL, mu_name TEXT NOT NULL, mu_description TEXT NOT NULL,
  form TEXT NOT NULL CHECK(form IN ('hybrid','textual')),
  status TEXT NOT NULL CHECK(status IN ('compiling','active','stale','needs_recompile','failed','retired')),
  revision INTEGER NOT NULL, capsule_sha256 TEXT NOT NULL, tree_sha256 TEXT NOT NULL,
  skill_files TEXT NOT NULL, env_fingerprint TEXT NOT NULL, schema_version INTEGER NOT NULL,
  compile_rules_version TEXT NOT NULL, source_session_id TEXT NOT NULL,
  source_generation TEXT NOT NULL, compile_id TEXT NOT NULL, artifact_dir TEXT NOT NULL,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_used_at TEXT);
CREATE VIRTUAL TABLE IF NOT EXISTS skillcache_capsules_fts USING fts5(
  capsule_id UNINDEXED, project_id UNINDEXED, skill_name, mu_name, mu_description,
  tokenize='porter unicode61');
CREATE TABLE IF NOT EXISTS skillcache_gates(
  capsule_id TEXT NOT NULL, round INTEGER NOT NULL, gate TEXT NOT NULL,
  passed INTEGER NOT NULL, report TEXT NOT NULL, created_at TEXT NOT NULL,
  PRIMARY KEY(capsule_id, round, gate));
CREATE TABLE IF NOT EXISTS skillcache_compiles(
  compile_id TEXT PRIMARY KEY, project_id TEXT NOT NULL, source_session_id TEXT NOT NULL,
  source_generation TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('queued','running','completed','failed','interrupted')),
  provider TEXT NOT NULL, model TEXT, rounds INTEGER NOT NULL DEFAULT 0, outcome TEXT,
  usage TEXT, receipt TEXT, started_at TEXT NOT NULL, finished_at TEXT);
CREATE TABLE IF NOT EXISTS skillcache_runs(
  generation TEXT PRIMARY KEY, session_id TEXT NOT NULL, capsule_id TEXT NOT NULL,
  provider TEXT NOT NULL, model TEXT, outcome TEXT NOT NULL, steps TEXT NOT NULL,
  deopts INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS skillcache_traces(
  generation TEXT PRIMARY KEY, session_id TEXT NOT NULL, provider TEXT NOT NULL,
  model TEXT, request TEXT NOT NULL, skill_refs TEXT NOT NULL, steps TEXT NOT NULL,
  usage TEXT, redaction TEXT NOT NULL, trace_sha256 TEXT NOT NULL, created_at TEXT NOT NULL);
```

Токены и стоимость читаются из `launches.usage`/`receipt` по `generation`; неизвестное
остаётся `null`. `recover` переводит `skillcache_compiles.status IN ('queued','running')` в
`interrupted`, а `skillcache_capsules.status = 'compiling'` в `failed` без
автоматического повтора, как `store.rs:329-344` для сессий.

### 4.7 `infra-ui` и `harness/web`

- `model.rs` (`State` :181): `skillcache: SkillCacheState` (режим, текущая фаза,
  счётчики); реестр — scoped-ключи `bind_scope("skillcache-registry", …)`.
- `session/prefs.rs` (`PREF_FIELDS` :5): `skillcache-mode`, `skillcache-compiler-model`;
  `session/composer.rs`: поле `skillcache` в `POST /api/sessions` и follow-up.
- `harness/web/index.html`: чип `#option-skillcache` (по умолчанию «Off») и панель
  `#panel-skillcache`; `<section class="view" id="skill-capsules-view" hidden>` после
  `#create-skill-view`.
- `session/navigation.rs` (:578): вкладки группы skills
  `["skills", "skill-capsules", "create-skill"]`; `view_label` — «Skill capsules».
- `session/advisory.rs`: пилюля предпросмотра (`Warm: <μ>`, `Miss: cold`, `Stale`).
- `run/outcomes.rs` (`TABS` :146): шестая вкладка `skillcache`, скрыта на cold-запусках;
  `run/stream.rs`: карточки фаз `needs_bindings` → `your_steps` → `complete` и
  `program_failed` (шаг, инструкция, диагностика; без кода программы).
- `management/capsules.rs` (новый): список и инспектор `μ, B, P, π, V`, история гейтов и
  раундов, действия requalify/invalidate/delete, амортизация (`null` → «—»).
- `session/results.rs`, `knowledge/usage.rs`: группа SkillCache (hits/misses/deopts,
  стоимость компиляции отдельно от ходов).
- `eval_viewer.rs` (:251): конфигурации `cold`, `skillcache_executor`, `skillcache_agent`,
  `raw_trace`, `written_procedure`, `skillcache_no_replay`; строки метрик `model_calls`,
  `deopt_rate`, `list_price_usd`, `compile_usd`, `breakeven_reuses`.
- `commands.rs`: палитра — переход к «Skill capsules».
- `harness/web/app.css`: цвета только токенами `:root` и `:root[data-theme="dark"]`.

### 4.8 `infra-eval`

`src/skillcache.rs` и операция `infra-engine eval skillcache-ab seal|run|report` (семейство
`skillcache-ab` добавляется в `infra-cli/src/lib.rs:877-881`):

- Протокол `skillcache-eval-protocol-v1` поверх `outcome::validate_design`/`seal`/
  `run_complete`/`summarize_complete`; изоляция верификатора — `isolation::prepare`/`probe`.
- Рукава: `cold`, `raw_trace`, `written_procedure`, `skillcache_agent` (капсула без
  executor-а), `skillcache_executor`, `skillcache_no_replay` (только Gate 1+2). Пробы:
  `exact`, `instance`, `rename`, `reword`. Матрица `(compile_model, executor_model)`.
- Метрики: `input`, `output`, `cache_read`, `cache_write` токены по провайдерам, число
  вызовов модели (дедупликация `message.id`), латентность, `list_price_usd` по запечатанной
  таблице цен в протоколе, частота деоптимизаций, амортизация: экономия на повтор,
  break-even (`null`, если экономии нет), чистая экономия при `K` повторах, bootstrap CI.
- `bench::ScriptedProvider` — детерминированные регрессии в CI без реального провайдера.
- Отсутствие токенов (Cursor) или `cache_write` (Codex) — `null` и показ покрытия, не ноль.

### 4.9 Документация и changelog

- Корневой [CHANGELOG.md](../../CHANGELOG.md): запись на каждую веху (изменения в
  `runtime-rust/` требуют её по `infra-tooling/src/changelog.rs`).
- [harness/README.md](../../harness/README.md): раздел SkillCache (режимы, opt-in трассы,
  требования к песочнице, ограничения провайдеров).
- Новый `docs/SKILLCACHE-EVAL.md` в M5 (протокол и правило приёмки).
- Журналы изданий не меняются: правки изданий не планируются.

## 5. Дорожная карта M0–M5

| Веха | Зависит от | Результат | Изменение поведения |
| --- | --- | --- | --- |
| M0 Контракты | — | crate `infra-skillcache`: схема, Gate 1, package, fingerprint, ресурсы компилятора; фикстуры | нет |
| M1 Executor и runtime | M0 | `skillcache-run`, `skillcache-tool`, фильтр окружения, multicall + guardian, `Spec.network`, `sandbox::gate` | новые CLI-команды |
| M2 Observe | M0 | `TraceRecorder`, `skillcache_traces`, режим `observe`, документированный opt-in | только при opt-in |
| M3 Compile и гейты | M1, M2 | компилятор, Gate 2/3, раунды, текстовый фолбэк, очередь, `skillcache_compiles`, HTTP compile/list/get, SSE `skillcache_capsule` | только по явному действию |
| M4 Warm | M1, M3 | hit/miss, промпт, стадирование, мост `run-log.jsonl`, деоптимизация, вкладка запуска | только при `auto`/`warm_only` |
| M5 Оценка и управление | M3, M4 | протокол `infra-eval`, вид «Skill capsules», eval viewer, Suite 12 E2E, флаги для выбора по `μ` и порогов перекомпиляции (выключены) | нет по умолчанию |

### M0 — контракты (1 PR)

Срезы: `Cargo.toml` crate + `Cargo.lock`; `capsule.rs`, `gate1.rs`, `package.rs`,
`fingerprint.rs`; `resources/compile/`. Готово, когда фикстура на каждое нарушение 3.1
отклоняется с ожидаемым кодом, `warm/` не содержит `E` (проверка дайджестом файлов), а
`cargo clippy --workspace --all-targets -- -D warnings` чист. Риск: расхождение схемы с
текстом статьи — схема помечена `schema_version: 1`, изменения через новую версию.

### M1 — executor и runtime (2 PR)

PR 1: `state.rs`, `executor.rs`, `tool.rs`, `env_filter.rs`, CLI-группа и multicall,
тест guardian. PR 2: `infra-host` `Spec.network` + `sandbox::gate` + тесты argv для
bwrap/SBPL/Windows. Готово: машина состояний проходит сценарии 6; запись состояния до
программы доказана тестом с аварийным завершением программы; на Windows инструменты —
`.exe`. Риски: guardian внутри песочницы провайдера (Codex seatbelt/landlock) — фикстурный
тест под bwrap; различия Windows job objects.

### M2 — observe (1 PR)

`TraceRecorder`, tee, попарное сведение трёх провайдеров на записанных фикстурах
событий, редакция, лимиты, `skillcache_traces`, ключ сессии. Готово: на фикстурах в трассе
нет reasoning и секретов (точные значения из окружения запуска заменены), при `off` ничего
не пишется. Риск: изменение контракта приватности — README и CHANGELOG в том же PR.

### M3 — компиляция и гейты (2–3 PR)

PR 1: `CompilerLaunchAdapter`, структурированный ответ, boundary-probe, `skillcache_compiles`.
PR 2: Gate 2/3, раунды, фолбэк, `skillcache_gates`, очередь с lease и `recover`. PR 3: HTTP
и SSE. Готово: компилятор фикстурного провайдера не может прочитать проект (probe), гейты
не вызывают модель (счётчик запусков провайдера = 0), Gate 3 собирает все последующие
сбои за один проход. Риски: Windows требует elevated Codex даже для Claude/Cursor;
стоимость компиляции — видна отдельно.

### M4 — warm (2 PR)

PR 1: hit/miss, `prompt.rs`/`context_usage.rs`, `ProjectLaunchAdapter`, стадирование,
allowlist, исключения (native Claude, Codex Manual). PR 2: мост `run-log.jsonl`,
`skillcache_runs`, вкладка запуска и карточки. Готово: на попадании промпт не содержит
`E`, кода программ и путей пакета (тест по ledger); на промахе поведение байт-в-байт
прежнее; `program_failed` для `V` не содержит `program_source_path`. Риск: Claude Bash
без песочницы может прочитать пакет — это не граница безопасности, а экономия токенов;
`E` физически вне warm-дерева.

### M5 — оценка и управление (2–3 PR)

`infra-eval/src/skillcache.rs`, запечатанный протокол, отчёт амортизации; вид «Skill
capsules»; eval viewer; Suite 12 в `harness/tests/browser_e2e.mjs`; флаги
`skillcache.retrieval_by_mu` и `skillcache.recompile_thresholds` (выключены). Готово:
CI-регрессии на `ScriptedProvider` зелёные; первый пилот — наблюдение, не основание для
включения по умолчанию.

## 6. Матрица проверки

| Инвариант | Проверка | Группа каталога |
| --- | --- | --- |
| Строгая схема, DAG, `outputs`/`result`, привязки ≠ содержимое файлов, секреты в `capsule.json` и `examples.json` | `infra-skillcache` unit `gate1_*` на фикстурах нарушений | `rust`, `msrv`, `windows-native` |
| `E` нет в warm-пакете и промпте | `package_warm_has_no_examples`, `prompt_hit_ledger_excludes_examples_and_sources`, `textual_hit_prompt_has_capsule_md_only` | `rust` |
| Состояние до каждой программы и переживание аварии | `executor_persists_state_before_program`, `state_attempt_survives_crash` | `rust` |
| `needs_bindings` / `your_steps` / `complete` / `program_failed` | `executor_transitions_*`, `continue_after_program_failed_requires_declared_outputs` | `rust` |
| 2 ненулевых выхода, таймаут, отсутствующий выход | `executor_deopt_after_two_nonzero`, `_timeout`, `_missing_output` | `rust` |
| Путь к коду только для `π` | `program_failed_verification_has_no_source_path` | `rust` |
| Нет повторного запуска упавшей программы | `tool_refuses_failed_program`, `continue_skips_failed_step` | `rust` |
| Фильтр секретов, только для программ | `env_filter_strips_key_token_password_case_insensitive`, `provider_env_untouched` | `rust`, `windows-native` |
| Один JSON на stdout | `tool_rejects_multiple_json_or_text` | `rust` |
| Assets read-only | `sandbox_gate_assets_read_only` (bwrap в CI), `host_mode_digest_unchanged` | `rust` |
| Guardian через multicall (`skillcache-tool` → программа) | `multicall_internal_guard_passthrough` (запуск `skillcache-tool` под hardlink-именем с фикстурой) | `rust`, `windows-native` |
| Сеть выключена в Gate 2 | `sandbox_gate_unshare_net_argv`, Windows TOML `network.enabled=false` | `rust`, `windows-native` |
| Компилятор не видит проект | `compile_boundary_probe_denies_project` (фикстурный провайдер) | `rust` |
| Гейты без модели | `gates_spawn_no_provider` | `rust` |
| Gate 3 подставляет `E` и продолжает; код `sandbox_denied` | `gate3_substitutes_examples_and_collects_downstream` | `rust` |
| 3 попытки → текст → исходный навык | `compile_rounds_then_textual_then_source` | `rust` |
| Трасса без reasoning и секретов; ничего при `off`/`warm_only` | `trace_*`, `trace_not_recorded_in_warm_only` на записанных событиях трёх провайдеров | `rust` |
| Ограничения warm-допуска | `warm_requires_edit_mode`, `warm_single_routed_skill`, `warm_skips_stale_capsule`, `warm_network_capability_mismatch_is_miss`, `warm_disabled_for_native_route`, `warm_disabled_for_codex_manual` | `rust` |
| Инвалидирование по любому файлу навыка | `fingerprint_detects_script_change` | `rust` |
| Мост `run-log.jsonl` и `history()` | `run_log_ingest_idempotent_emits_session_events`, `history_excludes_compiles` | `rust` |
| UI без инлайновых цветов, Wasm без нативных зависимостей | `assert_no_inline_hex_colors` в новых тестах; сборка Wasm | `rust`, `wasm` |
| Маршруты и SSE | тесты `http.rs` (`token`, query, `sse_filter_exempts_skillcache_capsule`), `api.rs` `sse_contract_tests` | `rust` |
| Ссылки и changelog | `infra-engine repo links`, `infra-engine repo changelog` | `links`, `changelog` |
| Браузер | Suite 12: чип и сохранение, вкладка `skillcache`, `program_failed`, вид «Skill capsules», CSRF, темы | вне каталога: `infra-engine repo build-ui`, затем `HARNESS_BROWSER_E2E=1 cargo test -p infra-harness --test browser_e2e` |

Новые группы в [install/check-catalog.json](../../install/check-catalog.json) не нужны:
crate попадает в `members = ["crates/*"]` и покрывается `rust`, `msrv`, `windows-native`;
`wasm` проверяет `infra-ui`. Все команды используют `--locked`, поэтому `Cargo.lock`
обновляется в том же PR, что и crate.

## 7. Вне объёма v1

- Выбор капсулы по `μ` без явного маршрута навыка и пороги автоматической
  перекомпиляции: только за выключенными флагами до собственного запечатанного измерения
  на held-out задачах ([правило стенда](../MEMORY-EVAL.md)).
- Автоматическая компиляция после каждого cold-прогона по умолчанию.
- Экспорт капсул в репозиторий (`.skillcache/`) и обмен между машинами.
- Warm для native Claude `/cmd`, Codex Manual, Fleet, Clash, SDD, System Orchestration,
  Infrastructure Creator.
- Пины кода проекта в капсуле (дрейф ловит деоптимизация и ручная переквалификация).
- Перенос в издания и генераторы Infrastructure-Creator.
- Утверждения об экономии без отчёта покрытия usage и без принятых задач (см.
  [cost-aware routing](2026-09-11-cost-aware-model-routing.md), п. 7).

## 8. Риски и открытые вопросы

1. **Покрытие токенов**: полный набор компонент есть только у Claude; Codex без
   `cache_write` и счёта вызовов, Cursor без токенов — отчёты показывают покрытие.
2. **Windows**: ОС-изоляция компиляции и гейтов использует elevated Codex (как для
   Creator/discovery в `sandbox.rs:205`) даже при запуске компилятора Claude; стадирование
   только `.exe`.
3. **Чтение пакета моделью**: не граница безопасности; `E` защищён физическим
   отсутствием в warm-дереве.
4. **Консервативность Gate 3**: `sandbox::gate` даёт минимальный корень без `/`, поэтому
   часть системных утилит хоста может получить `sandbox_denied` в Gate 3, даже будучи
   доступными в хостовом warm-запуске; это намеренно строже и отражается диагностикой.
5. **Сетевые программы**: `capabilities.network = "required"` выводится из трассы;
   ошибочный вывод даёт отказ Gate 3 (или промах на Codex без сети), а не тихое
   разрешение.
6. **Стоимость компиляции**: видна отдельно; амортизация не утверждается до пилота.
7. **Схема против текста статьи**: расхождения правятся новой `schema_version`, старые
   капсулы становятся `stale`.
8. **Компилятор только Claude в v1**: выбран ради гарантированного отключения shell;
   поддержка других компиляторов требует собственного режима без shell в их CLI.

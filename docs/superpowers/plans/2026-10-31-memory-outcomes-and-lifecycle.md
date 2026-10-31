# Memory Outcomes and Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. The user authorized implementation on 2026-10-31 after the planning stage, then authorized a local commit and preparation of the first private selective pilot. Push, publication and client deployment remain outside that request. Project delegation and single-writer rules take precedence over a skill's default execution method.

**Goal:** Повысить полезность памяти для правильно завершённых задач и сделать проверяемыми сохранение, повторное применение, консолидацию и обновление знаний.

**Architecture:** Канонические источники, Project Brain и Memory Bank сохраняют свои роли и порядок авторитета. Сначала расширяется существующий стенд до полного выполнения задачи с независимой проверкой; затем отдельные maintainer-only пилоты проверяют выборочный поиск между задачами, консолидацию, производное представление фактов и обслуживание актуальности. Никакой новый пилот автоматически не меняет устанавливаемый runtime или его defaults.

**Tech Stack:** Python standard library, существующие SQLite FTS5/BM25, Git и Codex CLI. Дополнительные базы, embeddings, NER-модели и сервисы в этот план не входят.

**Spec:** Пять приоритетов пользователя, изложенные в разделе «Требования» этого документа; действующие контракты `docs/MEMORY-EVAL.md`, `docs/CONTEXT-AND-MEMORY.md`, `docs/MEMORY-WRITE-PATH.md`, `docs/MEMORY-BENCHMARK-COST.md` и `PHP Core/project-brain/PROTOCOL.md`.

**Status:** Локальные инструменты реализованы и проверены по запросу пользователя от 2026-10-31; фактический результат приведён в конце документа. Реальные held-out/model измерения не запускались; runtime defaults не менялись.

**Baseline:** `/home/aliaksei/Desktop/AI-Infrastructure`, PR #84, ветка `fix/memory-field-measures-and-cli-2026-10-30`, HEAD `e8b8b39615fa34acf32510a50a45d2ecc7ac6653`, 2026-10-31. Перед исполнением повторно проверить HEAD, dirty state и актуальность перечисленных интерфейсов.

## Global Constraints

- Собственные инструменты репозитория остаются Python standard-library only; совместимость Python 3.9+ и платформ проверяется по текущей CI-матрице. Не переносить необязательные зависимости Harness в native runtime.
- Канонические источники выше Project Brain; Project Brain выше Memory Bank; производный индекс и отчёты не становятся новой authority.
- Не менять исходные клиентские checkout, Brain, Bank и настройки доверия. Экспериментальные операции выполняются на отдельных копиях; исходные образы проверяются до и после.
- Существующий endpoint first-touch, смысл legacy gold/recall reports, default `capsule_profile=recall` и доступность явного retrieve сохраняются.
- `automatic_consolidation=false` и `automatic_consolidation_verification=false` остаются defaults; для каждого флага требуется отдельное решение на основании измерения.
- Нельзя принимать эвристику поиска, допуска, бюджета или капсулы на метриках стенда релевантности. Нужен собственный task-outcome эксперимент на held-out задачах с протоколом, запечатанным до запуска.
- Механическая консолидация не равна проверке; agent attestation не равна human review; изменение digest не доказывает истинность утверждения.
- Секреты, персональные данные, сырые prompts/answers и клиентские журналы не входят в Git, Brain/Bank или агрегированные публичные отчёты. Приватные артефакты находятся вне clone, с ограниченным доступом.
- При будущих изменениях общего runtime сначала правится `PHP Core/`, затем выполняются `core_sync`, asset/mirror/policy и inventory-проверки по фактическому составу текущей ветки. Сгенерированные копии вручную не редактируются.
- Изменения общих `scripts/` и memory core получают запись в root `CHANGELOG.md`. Новые unittest-модули сразу регистрируются в соответствующем CI job и `scripts/check.py`; согласованность проверяется `tests.test_check`, а не откладывается до финального запуска.
- Один writer одновременно. Исследования и проверки могут идти параллельно; независимый reviewer не должен быть автором проверяемой реализации.

## Review Focus

1. Gold, verifier и labels недоступны агенту, а не просто отсутствуют в prompt: обычный workspace-write не запрещает чтение произвольного файла вне corpus. Проверки — задача 2.
2. Первое обращение к gold не отменяет cap/deadline; завершённый процесс без корректного результата не становится success. Проверки — задачи 1–3.
3. Источник сохранения и новая задача действительно различны; будущие знания, недавние файлы и одинаковый task ID не создают ложный cross-task результат. Проверки — задача 4.
4. Конфликт, отрицание, namespace/environment и lineage сохраняются при консолидации и разрешении версий; revision разных записей не сравниваются как общая шкала. Проверки — задачи 5–6.
5. Между review и записью может измениться источник; повтор, сбой promotion или архивирования не теряет доступное знание и не повышает доверие. Проверки — задачи 5 и 7.

## Требования

| Приоритет | Требуемый результат | Задачи |
| --- | --- | --- |
| 1. Правильность завершённой задачи | Полный run, независимая оценка ответа/патча, известный знаменатель, наблюдаемая стоимость | 1–4 |
| 2. Выборочный поиск между задачами | Измеряемый автоматический route за пределы own saves при сохранении компактного обычного контекста | 4, 8 |
| 3. Консолидация | Проверить существующий механизм на копиях реального Brain без потери ранних результатов и provenance | 5 |
| 4. Актуальные структурированные факты | Перестраиваемое представление фактов с идентичностью, областью действия, историей и явными конфликтами | 6 |
| 5. Актуальность | Очередь review и проверяемые действия confirm/replace/retire без автоматического освежения неподтверждённых утверждений | 7 |

Не входят: Redis/Neo4j/Mem0 migration, векторная база, новые TTL/decay/ranking-коэффициенты, семантический кеш готовых ответов, автоматическая правка policy/skills, установка в клиентские проекты.

## Подтверждённое исходное поведение

- `scripts/memory_outcome_ab.py:1007`: disposable corpus уже запускается с `workspace-write`, `--ephemeral`, `--ignore-user-config`; менять sandbox ради patch tasks заново не требуется. Упоминание read-only в docstring устарело.
- `Counter.feed` и `run_arm` заканчивают run при первом gold touch. `turn.completed`, сообщение агента и правильность результата пока не образуют независимый quality endpoint.
- `plant_saves` сохраняет результат на branch-task самого corpus. Существующий recall-set сам по себе проверяет same-task recall; он не является доказательством памяти между разными задачами.
- `PHP Core/AGENTS.md` уже требует явный retrieve перед сложной работой и существенными решениями. Предмет нового пилота — автоматический route, а не повторное добавление этого требования.
- Автоматический `recall` ограничен `own_saved_documents`; explicit retrieve использует full. Переменная `CONTEXT_CAPSULE_PROFILE` уже позволяет выбрать существующий профиль до prompt hook.
- `brain_runtime.consolidate(repository, *, apply=False, ticket=None, owner="local")` и оба флага консолидации существуют. События уходят только после появления доступного active chunk текущей revision вывода.
- `scripts/memory_fact_consolidation.py` — отдельный resolver по `fact` и explicit version/time; он не знает namespace/lineage и намеренно не подключён к runtime.
- `audit_bank`, `bank-reverify`, `bank-retire` и `bank-quarantine` существуют. `bank-reverify` — утверждение вызывающего, что он перепроверил знание; CLI сейчас не принимает actor/attestation.
- Изменившийся существующий источник допускает знание с `source_changed`; удалённая citation исключает его. Схемы chunk/dynamic-record закрыты: произвольные дополнительные поля недопустимы.

## Карта файлов и ответственность

Все пути ниже относительно repository root. Новые имена обозначают будущие файлы, а не уже существующие компоненты.

| Файл | Ответственность будущего изменения |
| --- | --- |
| `scripts/memory_outcome_ab.py` | Новый opt-in endpoint, manifest/coverage/report, cross-task case и подключение пилотного arm |
| `scripts/memory_outcome_verify.py` — новый | Контракт независимой проверки ответа/патча после завершения агента |
| `scripts/memory_outcome_isolation.py` — новый | Изолированная agent-visible область и проверка недоступности private verifier/gold |
| `scripts/memory_consolidation_pilot.py` — новый | Повтор существующего consolidation/promotion/compaction на копиях и сопоставление происхождения результатов |
| `scripts/memory_current_facts.py` — новый | Детерминированная производная проекция явно извлекаемых фактов |
| `scripts/memory_freshness_plan.py` — новый | Read-only review packet поверх существующего bank-audit; не самостоятельный mutator |
| `scripts/memory_selective_retrieve_pilot.py` — новый | Выбор существующего recall/full до одного prompt hook только в экспериментальном corpus |
| `tests/test_memory_outcome_ab.py` и отдельные `tests/test_memory_<component>.py` | Контракты перечисленных компонентов, fake-agent и публичные known-answer fixtures |
| `.github/workflows/ci.yml`, `scripts/check.py`, `tests/test_check.py` | Подключение новых тестов к реальным CI/local gates и проверка их паритета |
| `docs/MEMORY-EVAL.md`, `docs/MEMORY-WRITE-PATH.md`, `docs/CONTEXT-AND-MEMORY.md` | Актуальные контракты, ограничения и правила интерпретации результатов |
| `harness/src/harness/memory_benchmark_cost.py` | Существующий supplied-observation ledger; использовать его ограничения, не писать второй аналог |

## Зависимости и единица поставки

Первые три задачи образуют базу оценки. Задача 4 использует эту базу для настоящих save→later сценариев. Проектирование и локальные публичные fixtures задач 5–8 можно готовить независимо, но их принятие зависит от задачи 4 и собственного sealed measurement. Задача 9 завершает решение о переносе только подтвердившихся вариантов.

Каждая задача должна быть отдельным проверяемым изменением. Это не обязанность сразу складывать все направления в PR #84; конкретное разбиение будущей поставки определяется при исполнении с сохранением его текущего scope.

### Task 1: Полный run без изменения legacy endpoint

**Files:** modify `scripts/memory_outcome_ab.py`, `tests/test_memory_outcome_ab.py`, `docs/MEMORY-EVAL.md`.

**Interfaces:**
- CLI: `run --endpoint first-touch|task-complete`, default `first-touch`.
- `Counter(..., endpoint="first-touch")` и `run_arm(..., endpoint="first-touch")` получают только добавочный именованный параметр; прежние вызовы сохраняют смысл.
- Новый completion result различает событие первого gold touch, завершение CLI, завершение agent turn и результат последующей проверки. `completed` ещё не означает `correct`.

- [ ] Добавить fake-agent тест `test_task_complete_runs_past_gold`: gold touch → изменение файла → финальный ответ → `turn.completed` → exit 0; изменение выполнено, usage получен, first-touch diagnostic сохранён.
- [ ] Добавить проверки `test_task_complete_keeps_cap_at_gold`, `test_task_complete_timeout_after_gold`, `test_failed_or_unfinished_process_is_not_complete`: cap на gold обязателен, quiet process останавливается, failed event/нет completion/ненулевой exit не дают success.
- [ ] Запустить новые тесты против старого поведения и получить ожидаемый failure; legacy `test_the_run_stops_at_the_gold_touch_and_tells_hook_from_agent` должен остаться зелёным после реализации.
- [ ] Добавить endpoint без изменения legacy gold/recall verdict; исправить устаревший read-only docstring.
- [ ] Проверить `python3 -m unittest tests.test_memory_outcome_ab`: новый режим не останавливается на gold, старый по умолчанию останавливается как раньше; процесс и потомки убираются после всех terminal paths.

**Deliverable:** полный ограниченный run и отдельный статус completion, без новых runtime эвристик.

### Task 2: Независимый verifier и реальная изоляция gold

**Files:** create `scripts/memory_outcome_verify.py`, `scripts/memory_outcome_isolation.py`, `tests/test_memory_outcome_verify.py`, `tests/test_memory_outcome_isolation.py`; modify controller/tests из задачи 1.

**Interfaces:**
- Task input: `task_mode="answer"|"patch"`, непрозрачный `verifier_id`.
- Controller-only CLI: `--verifiers PRIVATE_MANIFEST`; он не передаётся агенту, hook, индексу или router.
- `verify_task(snapshot: Path, answer_artifact: Path, trusted_case: dict) -> dict` возвращает `status=passed|failed|unavailable`, `correct=true|false|null`, digests проверяющего и проверяемого результата.
- Для patch verifier выполняет доверенный argv без shell на отдельной копии результата после завершения агента. Для answer использует заранее заданную проверку условий/фактов; если честной детерминированной проверки нет, оценка помечается unavailable либо проводится отдельная слепая разметка по запечатанной rubric, а не подменяется поиском имени файла.
- `prepare_agent_view(corpus: Path, private_root: Path) -> dict` и `probe_agent_view(view: dict, sentinel: Path) -> bool` описывают проверенную границу читаемых файлов. Пилот Linux использует отдельный mount namespace/allowlist окружения; перед реализацией сопоставить с существующим `harness/src/harness/discovery_sandbox.py`, не импортируя его Harness-зависимости в stdlib инструмент. При недоступности границы private holdout не запускается. Нативная Windows-проверка из Linux не заявляется.

- [ ] Зафиксировать публичные fixtures: неверный ответ с правильным именем файла → false; неверный patch с завершённым turn → false; правильный patch → true; verifier error/timeout → null, а не false success.
- [ ] Проверить `test_agent_cannot_read_private_sentinel`: процесс из той же agent-visible области не может открыть gold/manifest или обходные symlink-пути. Проверить также отсутствие sentinel в command/env/corpus/events; одной этой проверки текста недостаточно для защиты чтения.
- [ ] Проверить, что агентская правка своих тестов не изменяет внешний verifier; каждый arm/repeat получает независимую копию одинакового base digest.
- [ ] Сначала реализовать и проверить публичные known-answer fixtures без моделей. Добавить отдельные timeout, статус toolchain-unavailable, проверку неизменности verifier и очистку после сбоя.
- [ ] Интегрировать verifier до удаления corpus; финальный ответ хранить только в приватном временном артефакте, агрегировать counts/digests, не ответ/gold.
- [ ] Запустить `python3 -m unittest tests.test_memory_outcome_verify tests.test_memory_outcome_isolation tests.test_memory_outcome_ab` и подтвердить все assertions. Недоступную OS-изоляцию показывать как limitation, не как пройденный blind holdout.

**Deliverable:** качество определяется независимо от самоотчёта агента и доступа к разметке.

### Task 3: Запланированный знаменатель, completion report и честная стоимость

**Files:** modify `scripts/memory_outcome_ab.py`, `tests/test_memory_outcome_ab.py`, `docs/MEMORY-EVAL.md`; read existing cost-ledger contract/tests, не добавлять зависимость Harness к stdlib CLI.

**Interfaces:**
- Новый CLI contract: `run --endpoint task-complete --protocol PRIVATE_PROTOCOL_JSON --verifiers PRIVATE_MANIFEST`. Для нового endpoint оба файла обязательны; fake-agent fixtures используют публичные эквиваленты. Legacy first-touch их не требует.
- Manifest `task-outcome-complete-v1` сохраняется атомарно до первого model process: task IDs, arms/repeats/order, endpoint, model/CLI/verifier/input digests, cap/deadlines, retry policy. Runtime/config digests фиксируются отдельно для каждого arm: заявленное различие treatment допустимо, любое незаявленное отклонение от seal — нет.
- Один task×arm×repeat — один логический slot; status и стоимость каждой попытки сохраняются отдельно. Recovery retries по умолчанию отсутствуют; повтор не удаляет предыдущую стоимость и не увеличивает число задач.
- `report --metric task-complete` — отдельный summary, не переименование gold/recall. Выводит planned/attempted/completed/verified/correct/incorrect/unknown и coverage.
- Host-reported input/output/cached tokens и elapsed отделены от полной стоимости provider attempts. Неизвестные скрытые retry, неполный usage и USD остаются unknown. Cached input — подмножество input; bytes не токены.

- [ ] Добавить test-oracles: interruption после 3 из 8 slots сохраняет planned=8 и unattempted=5; failure с usage не исчезает; retry сохраняет обе попытки; missing usage не превращается в ноль.
- [ ] Добавить тесты отчёта: unknown не исключается из знаменателя ради «100% по известным»; быстрый gold touch при неверном итоге не даёт положительного quality verdict; legacy reports прежние.
- [ ] Реализовать checkpoint до запуска и после каждой terminal attempt, отдельный completion summary и coverage; незавершённый план или неизвестная оценка запрещают финальный causal verdict.
- [ ] Финальные rates считать по заранее назначенным slots; timeout/cap/agent failure означают невыполнение в заданном бюджете. Неустановимая оценка/инфраструктурная неопределённость видна отдельно и даёт insufficient, а не успешное пересечение.
- [ ] Учитывать повторы внутри task: единица парного анализа — задача, не отдельный повтор. Не выбирать лучший ответ из нескольких post hoc.
- [ ] Сохранить ограничения существующего ledger: `build/query/reader/judge`, PID CLI не provider-attempt ID. Локальный deterministic verifier не объявлять complete zero-attempt judge; его время учитывать отдельно. При отсутствии полного покрытия не рассчитывать total cost/savings/cost-per-correct как известные.
- [ ] Запустить `python3 -m unittest tests.test_memory_outcome_ab tests.test_memory_benchmark_cost`; вручную рассчитанные public oracles должны совпасть с отчётом.

**Deliverable:** проверяемый полный outcome и наблюдаемая стоимость без скрытого отбора удачных запусков.

### Task 4: Настоящий save→later и запечатанный протокол

**Files:** modify `scripts/memory_outcome_ab.py`, `tests/test_memory_outcome_ab.py`, `docs/MEMORY-EVAL.md`; extend `tests/test_memory_native_roundtrip.py` для публичной механики. Приватные inputs/protocol/results — вне clone и agent-visible области.

**Interfaces:**
- Case связывает `source_task_id`, `served_task_id`, snapshot digest, save receipt/result identity, `save_provenance=observed|generated|authored`, момент сохранения и вопроса.
- Same-task regression остаётся отдельным классом; cross-task case требует разных task UUID, а не только разных внешних строк.
- `plant_saves` сохраняет legacy поведение для старых sets; новый case восстанавливает сохранение на source task и обслуживает вопрос на другой задаче. Authored injection честно остаётся synthetic, не «реальным сохранением пользователя».

- [ ] Добавить публичный integration case: save на A → новая задача B → новая сессия без recent-file подсказки → знание доступно full retrieve, но не объявлено own save задачи B.
- [ ] Проверить source changed/missing, новый ответ после смены решения, повтор result_id после частичного сбоя, одноимённые сущности и отсутствие подходящего знания; не добавлять их в prompt как ответные подсказки.
- [ ] Подготовить отдельные observed или реально generated session-A сохранения через существующий native-result/record-result, с доказательством сохранения и provenance. Само сравнение arms начинается с одинакового уже сохранённого base; стоимость создания памяти учитывается и отдельно от ответа B.
- [ ] Использовать реальные поздние состояния или явно маркированное replay с intervening tasks; не выдавать простую задержку clock на минуту за long-term проверку.
- [ ] До held-out запуска запечатать одну baseline/treatment пару, selection/exclusion, primary endpoint correctness, quality margin, secondary benefit, sample/power rationale, repeats, budgets, retries, stop/unknown policy и digests inputs/variant/verifiers/runtime. Tuning и holdout разделены; просмотренный для подстройки holdout больше не held-out.
- [ ] Добавить preflight-тест: несовпадение запечатанных digests отклоняет run до модели. Не устанавливать sample size или quality margin задним числом; точные значения станут частью конкретного протокола, а не выводятся из прежнего 20-pair navigation threshold.
- [ ] Запустить `python3 -m unittest tests.test_memory_outcome_ab tests.test_memory_native_roundtrip`; реальный model run остаётся отдельным будущим действием, не выполненным в этом плане.

**Deliverable:** набор и протокол действительно различают same-task recall, cross-task recall, synthetic и observed evidence.

### Task 5: Измеренный пилот существующей консолидации

**Files:** create `scripts/memory_consolidation_pilot.py`, `tests/test_memory_consolidation_pilot.py`; extend существующие consolidation tests в `PHP Core/project-brain/tests/test_runtime.py`; update `docs/MEMORY-WRITE-PATH.md`. Сначала без изменения алгоритма/defaults.

**Interfaces:**
- Proposed CLI: `python3 scripts/memory_consolidation_pilot.py --root SNAPSHOT --out PRIVATE_REPORT --variant baseline|consolidate|consolidate-verify`.
- Пилот всегда создаёт собственную копию; не принимает исходный root как место apply. Вызывает существующие consolidate/turn/promotion/compact, не пишет records/chunks напрямую.
- `baseline`: оба флага false; `consolidate`: только consolidation true; `consolidate-verify`: оба true. Одинаковые promotion/compaction settings и snapshots в сравниваемых arms.

- [ ] Добавить fixtures с ранним/поздним исходом, отрицанием/исключением, непроверенным event, чужим owner, private/restricted event, независимым resolved finding, поздним event и `UTF-8`/`PHP 8.2`.
- [ ] Проверить: observed summary не становится verified от самого объединения; agent-attested summary не называется human-reviewed; повтор без новых данных не создаёт новый finding/chunk/revision.
- [ ] Проверить promotion failure и fresh clone из tracked файлов после compact: история остаётся доступной, пока active chunk нужной revision не существует; early outcome не теряется после замещения.
- [ ] Запустить dry-run, затем существующий apply/turn на копиях; сохранить diff, соответствие event UUID→summary revision→chunk, проверки и статусы каждого шага. Повторный запуск после прерывания должен восстанавливаться без дубликатов.
- [ ] Подключить полученные образы как отдельные treatments full-outcome задач 1–4. Снижение количества events/chunks или лучшая доставка — диагностика, не verdict.
- [ ] Запустить `python3 -m unittest tests.test_memory_consolidation_pilot` и `(cd 'PHP Core/project-brain/tests' && python3 test_runtime.py)` после реализации; независимо решить вопрос для каждого флага на его собственном sealed результате.

**Deliverable:** измерение существующего write-path, а не заранее включённая автоматическая verification.

### Task 6: Производная проекция актуальных фактов

**Files:** create `scripts/memory_current_facts.py`, `tests/test_memory_current_facts.py`; update `docs/MEMORY-WRITE-PATH.md`. `scripts/memory_fact_consolidation.py` и closed runtime schemas сначала не менять.

**Interfaces:**
- CLI: `python3 scripts/memory_current_facts.py --root SNAPSHOT --selectors SELECTORS_JSON --as-of UTC_INSTANT --json`.
- `collect_candidates(root: Path, selectors: list, *, as_of: str) -> list`; `resolve_current(candidates: list, *, namespace: str, as_of: str) -> dict`.
- Начальные selectors извлекают scalar из публичного JSON по JSON Pointer или ссылаются на явную структурированную запись поддержанного формата. Selectors содержат identity/alias/extraction path, но не отдельную копию «истинного значения». PHP/config/env не исполняются; `.env` и секреты не читаются. Свободный неподдержанный текст остаётся unclassified.
- Identity: repository/source-frame + subject + predicate + применимые qualifiers, включая environment. Branch name сам по себе не даёт applicability. Проекция хранит claim, provenance/digest, record UUID/revision при наличии, validity и причину состояния `current|ambiguous|stale|unclassified`.

- [ ] Добавить fixtures: MySQL→PostgreSQL с доказанной заменой; равносильные несовместимые утверждения; разные projects/environment; unrelated revisions; UTC/timezone варианты; поздняя запись с задним valid_from; удалённый/изменённый источник.
- [ ] Зафиксировать правила: revision сравнима только внутри lineage; explicit conflict не исчезает от updated_at; canonical source выше пересказа; created/recorded after snapshot недопустим; одинаковая сила и несовместимость → ambiguous.
- [ ] Проверить reorder invariance, отказ от private/secret/escaping path, сохранение отрицания и scope. Удаление отчёта и перестроение должны воспроизводить тот же результат без потери истории.
- [ ] Реализовать только read-only производное представление. На первом этапе достаточно JSON-отчёта; новую SQLite-схему, обязательную fact-схему в Brain/Bank и NER не добавлять без отдельно измеренной необходимости.
- [ ] Запустить `python3 -m unittest tests.test_memory_current_facts tests.test_memory_fact_consolidation`.
- [ ] До передачи этой проекции в капсулу оформить отдельный treatment с теми же namespace/authority checks и sealed full-outcome из задачи 4. Offline правильность resolver не разрешает runtime интеграцию.

**Deliverable:** типизированная текущая картина и история без второй authority и ложного объединения сущностей.

### Task 7: Обслуживание актуальности без массового reverify

**Files:** create `scripts/memory_freshness_plan.py`, `tests/test_memory_freshness_plan.py`; update `docs/MEMORY-WRITE-PATH.md`, `docs/CONTEXT-AND-MEMORY.md`. Начальный pilot не меняет `brain_runtime.py` или chunk schema.

**Interfaces:**
- CLI: `python3 scripts/memory_freshness_plan.py --root SNAPSHOT --json`.
- `build_review_packet(audit: dict, *, snapshot_digest: str) -> dict` структурирует существующий `bank-audit --provenance --json`: chunk/problem/source fingerprints, требуемая проверка, допустимое действие и причина. Read-only по умолчанию и по контракту.
- Таблица действий: still true после проверки → `bank-reverify`; изменённый факт → текущий Brain workflow и superseding promotion; ceased/no successor → `bank-retire`; unsafe → `bank-quarantine`; ambiguity → сохранить конфликт и разбирать.

- [ ] Добавить тесты отсутствия мутаций audit/packet, source_changed/missing/overdue/duplicate случаев; ни возраст, ни частота чтения не доказывают неверность или истинность.
- [ ] Проверить детерминированность очереди и идемпотентность review packet на неизменившемся snapshot; поддержать явный лимит обрабатываемых кандидатов и признак truncation, не скрывая необработанную часть. Не запускать полный duplicate audit в каждом prompt hook.
- [ ] На копии перечитать claim и источник, выполнить независимую проверку, сохранить actor/attestation/evidence в приватном отчёте, затем применить одну существующую операцию и повторить audit/retrieve. Не обозначать существующий bank-reverify как human review: такого параметра сейчас нет.
- [ ] Проверить reverify не возрождает retired chunk, retire сохраняет историю/обе стороны замещения, несвязанный overdue chunk не блокирует целевое действие; использовать существующие runtime tests BankRetire/ProvenanceAudit/ChunkSourceDigest/ChunkCalendar.
- [ ] Запустить `python3 -m unittest tests.test_memory_freshness_plan tests.test_memory_temporal_delivery tests.test_memory_retained_citations` и affected runtime tests после реализации.
- [ ] Unattended apply оформить отдельным последующим изменением только после пилота: добавить expected chunk/source images и actor/evidence contract к существующему mutator, проверять их под его mutation lock. Проверка hashes снаружи перед отдельным CLI не является CAS; изменившийся после review источник требует повторной проверки. Дополнительные поля должны пройти совместимую schema/parity миграцию; внешняя очередь не становится authority.

**Deliverable:** управляемый цикл обнаружить→проверить→подтвердить/заменить/закрыть. Первоначальная поставка автоматизирует обнаружение и подготовку, не выдаёт переписывание digests за проверку содержания.

### Task 8: Пилот выборочного автоматического cross-task поиска

**Files:** create `scripts/memory_selective_retrieve_pilot.py`, `tests/test_memory_selective_retrieve_pilot.py`; modify experiment integration в `scripts/memory_outcome_ab.py`, его тесты и `docs/MEMORY-EVAL.md`. Runtime/AGENTS/defaults не менять.

**Interfaces:**
- `choose_profile(query: str, rules: dict) -> str` возвращает `recall|full`; rules — одна версионированная детерминированная routing policy, зафиксированная до holdout. На tuning этапе формализовать retrospective/decision cues RU/EN; router не получает gold, labels, ответы или future transcript.
- Maintainer wrapper читает текущий запрос только для намерения, использует существующую санитизацию, задаёт `CONTEXT_CAPSULE_PROFILE` и вызывает один исходный prompt hook внутри disposable corpus, сохраняя host/session/transcript/task/gate.
- Добавочный arm `selective`: recall на обычных запросах, существующий full-поиск на выбранных. Это выборочный профиль в одном hook, а не дополнительный explicit MCP call. Explicit retrieve остаётся одинаково доступен всем arms.

- [ ] Сначала проверить policy-invocation baseline на tuning cases: explicit retrieve уже обязателен по AGENTS, поэтому не добавлять treatment специальную подсказку «пользоваться памятью».
- [ ] Добавить тесты: wrapper выключен → исходный hook без изменений; исторический запрос → full; новая обычная задача → recall; router error/timeout → recall с отдельной диагностикой; смена gold/labels не влияет на route.
- [ ] Проверить один hook/render/session receipt, неизменный served task, сохранённые budget/gate и source admission; не расширять own_saved_documents и не подставлять source task как текущую задачу.
- [ ] Проверить restricted-owner/privacy, конфликты, повтор той же revision, compaction/new revision, packed-out items и source-changed/missing по существующим контрактам. Общие team записи не запрещать только из-за другого автора.
- [ ] Не обещать environment isolation от слова в query: у текущего retrieval нет отдельного environment admission. Scope-sensitive cases должны либо получить точный контекст/уточнение, либо явный конфликт; новая фильтрация по environment — отдельное будущее изменение после задачи 6.
- [ ] Запустить `python3 -m unittest tests.test_memory_selective_retrieve_pilot tests.test_memory_outcome_ab`; подготовить sealed сравнение recall↔selective, full оставить диагностическим comparator с отдельным статистическим статусом.
- [ ] На полном outcome считать assigned-arm результат, стоимость, TP/FP/FN/TN исторической потребности, router errors, пустой поиск, gate skip, rendered delivery и неверное применение памяти. Silent fallback не исключать из treatment как будто его не было.

**Deliverable:** измеренный выбор момента широкого поиска без нового ranking, отключения gate или постоянного расширения каждой капсулы.

### Task 9: Решение о внедрении и проверки поставки

**Files:** reports/protocols задач 4–8; обновления соответствующих docs. Canonical runtime/config/tests/schemas и root `CHANGELOG.md` изменяются только для отдельного подтверждённого переноса.

- [ ] Для каждого кандидата сопоставить sealed protocol и фактический run: variant/input/runtime digests, holdout provenance, завершённость плана, корректность проверки, coverage стоимости и исходные snapshots.
- [ ] Сначала оценить correctness и ошибки применения устаревших знаний; затем заявленный secondary benefit. Нельзя оправдывать худшее качество меньшими токенами вне заранее заданного quality contract.
- [ ] Кандидат получает `accept|reject|insufficient` по своему протоколу. Negative/insufficient сохраняются; вариант не подстраивается на том же holdout. Отдельно решаются router, consolidation, verification и доставка facts.
- [ ] При accepted переносе править `PHP Core/` канон и только относящиеся к нему тесты/конфигурацию, сохранить понятный opt-out/rollback до прежнего поведения; синхронизировать реальные editions/generator assets через существующие scripts.
- [ ] Выполнить affected unit/integration checks и edition DOD, `core_sync.py --check`, `build_mirrors.py --check`, `asset_parity.py --check`, `policy_lock.py --check`, `install_accelerator.py --verify-inventories`, а затем текущие CI-mirroring checks через `scripts/check.py`. Уточнить CLI конкретных sync/check scripts перед исполнением.
- [ ] Если новые distribution files требуют inventory, не staging всего dirty tree: включать только соответствующие пути при явно авторизованной поставке. Отделить локальные проверки от CI и native Windows доказательств.
- [ ] Независимый reviewer проверяет реализованные boundaries, результат измерения и итоговый diff. Выход — проверяемый отчёт, а не автоматический merge/deploy.

## Общая матрица приёмки

| Сценарий | Ожидаемое поведение |
| --- | --- |
| Знание нужно значительно позже | Задача решена правильно; видна provenance сохранения и фактической доставки/проверки |
| Сохранение другой задачи | Source/served UUID различны; default own-saves membership не подменён |
| Решение изменилось | История сохранена; старое решение не выдано без предупреждения как действующее |
| Одинаковое имя в двух проектах/окружениях | Нет молчаливого объединения; ambiguity/уточнение вместо произвольного выбора |
| Две равносильные несовместимые записи | Явный conflict, не выбор «самой новой» по несопоставимой revision |
| RU/EN перефразирование и отрицание | Смысл, исключение и scope сохранены; отсутствие знания допускает отказ |
| Память не относится к новой задаче | Нет необоснованного расширения контекста или переноса старого решения |
| Promotion/compact/retry прерваны | Нет потери доступного исходного знания, дубликатов или ложной verification |
| Агент закончил, но результат неверный | completed отдельно, correct=false; gold touch не считается успехом |
| Verifier/usage/изоляция неизвестны | Unknown и недостаточное доказательство видимы; финальная выгода не выдумана |

## Порядок исполнения и первая законченная поставка

1. Выполнить задачи 1–3: full completion, внешний verifier с проверенной изоляцией, manifest/quality/cost reporting.
2. Выполнить задачу 4: подлинные save→later/cross-task cases и запечатанный протокол. Это первая самостоятельная поставка; она полезна без изменения памяти runtime.
3. Подготовить задачи 5–7 на публичных fixtures и копиях; провести отдельные реальные пилоты только после готовности outcome-инфраструктуры.
4. Подготовить и измерить задачу 8 при одинаковых инструментах/политике всех arms.
5. По задаче 9 переносить только подтвердившиеся изменения. Embeddings/graph/cache остаются за пределами этого решения.

## Самопроверка плана

- Все пять пользовательских приоритетов имеют задачи, критерии и зависимости.
- Уже существующие workspace-write, explicit retrieve, consolidation, audit/reverify/retire, source links и cost ledger не предлагаются как отсутствующие возможности.
- Чётко отделены completion, correctness, rendered delivery, source verification и полная стоимость.
- Выделены gold-read isolation, same-task/cross-task, source race и закрытые schema; они не скрыты за общими «проверить безопасность».
- Исходный список выше задаёт критерии реализации и будущей экспериментальной приёмки; фактический статус приведён ниже. Реальные модельные результаты и новые defaults не заявлены.
- После планирования пользователь отдельно разрешил реализацию. Коммиты, публикация и клиентское внедрение не выполняются.

## Результат локальной реализации — 2026-10-31

| Задачи | Поставленный результат | Граница доказательства |
| --- | --- | --- |
| 1–3 | `task-complete`, независимый answer/patch verifier, проверяемая Linux/bubblewrap изоляция, CLI `seal → run → report`, неизменяемый план slots и учёт неполных исходов/затрат | Проверено публичными тестами с fake agent и настоящими изолированными процессами; реальные модели не вызывались |
| 4 | Разные source/served UUID, native save→replay, импорт captured receipt, sealed inputs/runtime/verifier digests | Инъекция остаётся `authored`; receipt/hash не удостоверяет авторство; реальное позднее применение на клиентских задачах не измерено |
| 5 | Три варианта консолидации на физических копиях, provenance/validation/idempotence отчёт, frozen treatment для полного outcome | Механическая консолидация не подтверждает истинность; оба runtime defaults выключены |
| 6 | Read-only проекция scoped фактов из публичного JSON, UTC/validity/lineage/conflict/stale проверки, сохранение истории | Производный отчёт, без интеграции в капсулу и без новой authority |
| 7 | Реальный provenance audit на копии и детерминированная ограниченная очередь перепроверки | Нет автоматического reverify/retire; проверка содержания и actor/evidence остаются отдельным действием |
| 8 | Экспериментальный selective wrapper одного исходного hook, RU/EN rules, sealed arm и диагностика маршрутизации | Только opt-in пилот; качество маршрутизации на реальных held-out задачах не установлено |
| 9 | Регистрация тестов в CI/check.py, документация, root changelog, независимая проверка итогового изменения | Перенос эвристик в установленный runtime требует отдельного `accept`; текущих доказательств для него недостаточно |

- [x] Целевые проверки: **192 tests OK**, Python 3.14.8, 23.447 s, без skips.
- [x] Совместимость новых инструментов с Python 3.9: **113 tests OK**, 11.574 s, без skips; реально исполнены isolated answer/patch проверки.
- [x] Независимый финальный reviewer завершил три прохода и две изолированные репродукции. Оба подтверждённых Important замечания исправлены с RED→GREEN регрессиями: сохранение recent-file baseline в consolidation treatments и bounded/no-follow сбор telemetry после выхода агента. Последующая read-only проверка исправлений и relocated Python изоляции подтвердила закрытие обоих замечаний; новых блокеров не обнаружено.
- [x] `git diff --check`: без ошибок.
- [x] Полный CI-mirroring прогон `scripts/check.py --jobs 3 --base origin/docs/memory-contracts-2026-10-30`: **245 passed, 0 failed, 6 skipped**, exit 0, 1773.5 s. Пропуски: отсутствующий ShellCheck, две проверки необязательного `scripts/qa/.venv` и три Windows-only набора. Локальные parity/mirrors/inventories/policy-lock/budget/links/changelog и тесты всех четырёх редакций прошли. Изменённые модули дополнительно проверены окончательными целевыми прогонами выше; remote CI не запускался.
- [ ] Реальные held-out сравнения и измерение полной provider-стоимости: не запускались; для каждого treatment нужен собственный запечатанный набор и протокол.
- [ ] Включение новых defaults, native Windows и remote CI: не выполнялись и не выводятся из локальных результатов.

Журнал исполнения и RED/GREEN доказательства сохранены локально в игнорируемой `.superpowers/sdd/2026-10-31-memory-outcomes-and-lifecycle/`; commits/push/merge/deploy не выполнялись. Приватный full-outcome endpoint требует Linux/bubblewrap, использует только публичный snapshot Git и отклоняет history-dependent `ticket` arm. Доверенный patch oracle обязан изолировать исполнение кандидатного кода от своих labels; универсальная безопасность произвольного oracle не заявляется.

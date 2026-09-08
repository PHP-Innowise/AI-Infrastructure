# Open Source Kit · Kit 3

A searchable catalog of AI development tools, with a local web interface and a
CLI for Agent Skills. Kit 3 complements Infrastructure-Creator (Kit 1) and the
ready-made accelerator editions (Kit 2).

The catalog records compatibility claims, installation guidance and review
findings. **Vercel Skills manages actual skill installation, listing, updates
and removal.** Plugins, MCP servers, hooks and discovery indexes retain their
own installation guides. A collection is not automatically an installable skill.

## Quick start

Run from this repository. Python 3.9+ is sufficient for the catalog and web
preview. Skill management additionally needs Node.js **22.20.0+** and `npx`.

```bash
./kit3 serve
# Open http://127.0.0.1:8765

./kit3 find review --agent codex
./kit3 show caveman
./kit3 add caveman --agent codex --list
./kit3 add caveman --agent codex --target /path/to/project --dry-run
./kit3 add caveman --agent codex --target /path/to/project
./kit3 list --target /path/to/project
```

The browser provides search, category/client/review filters, resource detail
links and copyable commands. It does not execute commands on your computer.
`serve` binds to localhost and serves only a temporary catalog build, not the
repository. For static hosting, build and publish the resulting directory using
your existing hosting workflow:

```bash
./kit3 build --output output/kit3-site
```

There is no server API, database, remote font or frontend dependency. The web
catalog is generated from [resources.json](resources.json) and validated
[registry entries](registry/). Edit these sources, then rebuild.

## CLI

| Command | Behavior |
| --- | --- |
| `find [QUERY]` | Search the local catalog without network access |
| `find QUERY --remote` | Search the Vercel skills ecosystem |
| `show ID` | Show source, compatible clients, review and manual guidance |
| `add SOURCE` | Install Agent Skills through Vercel Skills |
| `list` | List project skills through the native manager |
| `update [NAME...]` | Update project skills; `--global` explicitly selects global scope |
| `remove [NAME...]` | Remove skills through the native manager |
| `record ID...` | Record a catalog selection and review in `.kit3-manifest.json` |
| `list --recorded` | Read the separate project selection record |
| `build` / `serve` | Build a static site / preview it on localhost |

`find` supports `--agent`, `--category`, `--review` and `--json`. `show` and
`list` support `--json`. Remote search uses the native search interface; local
catalog filters cannot be combined with `--remote`.

`add` accepts a supported catalog ID, `owner/repo`, an HTTPS source URL, a local
skill directory, or a local Git `file://` URI. Catalog shortcuts currently expose
the **skills-only** paths of Caveman and Superpowers. Their full plugins, hooks
or proxy installations are separate choices. An unsupported catalog ID prints
an error directing you to `show`; its prose guide is never executed.

```bash
# Discover components before choosing which ones to install.
./kit3 add vercel-labs/agent-skills --list
./kit3 add owner/repo --skill skill-name --agent codex --copy --target /path/to/project

# Repeat --agent / --skill for an explicit selection.
./kit3 add owner/repo --skill first --skill second --agent codex --agent cursor

./kit3 list --json --target /path/to/project
./kit3 update skill-name --target /path/to/project --dry-run
./kit3 update skill-name --target /path/to/project
./kit3 remove skill-name --agent codex --target /path/to/project
```

Commands that use the native manager print the exact argv and target on stderr;
JSON output stays on stdout. `--dry-run` performs no downloads, subprocess calls
or writes. Otherwise `npx` may download the pinned manager **`skills@1.5.23`**.
Native prompts are preserved; `--yes` accepts them. The manager owns its lock
files, copy/symlink behavior and collision handling. See the
[Vercel Skills documentation](https://github.com/vercel-labs/skills).

`update` always passes an explicit project or global scope. This manager version
redetects clients when updating and does not preserve an explicit `--copy`
choice; inspect the resulting client paths. Native exit status is forwarded,
but a zero status alone is not a verified installation receipt. Use `list` and
inspect the files for the project. Removal concerns the selected skills, not
arbitrary files a skill or plugin previously generated.

The manager version pin does **not** pin the installed resource. Choose the
appropriate immutable source revision where supported and inspect the native
lock state. Changes to installed content need a new review.

## Selection records and existing scripts

The original selector remains compatible:

```bash
python3 scripts/install_open_source_kit.py --list
python3 scripts/install_open_source_kit.py --select caveman --target /path/to/project --dry-run
./kit3 record caveman --target /path/to/project --pin caveman=EXACT_REVIEWED_REF
./kit3 list --recorded --json --target /path/to/project
```

`record` selects catalog entries; it installs nothing. `.kit3-manifest.json`
contains selection dates, supplied pins, guidance, risk notes and a review
snapshot. It is separate from Vercel's installed-skill inventory. Existing pins
survive reselection. Neither a supplied pin nor a selection proves which files
are installed. `remove` does not erase that historical selection.

Manifest reads reject malformed data and symbolic links. Writes use a temporary
file and atomic replacement relative to an opened target directory, preserving
external hard-link targets. Targets with symbolic-link components are refused.
These protections cover Kit 3's own manifest; they do not replace the external
manager's filesystem policy.

The optional legacy `--refresh` reads the current GitHub README. Extracted
commands are **unreviewed suggestions**, even when only one is found. They are
saved separately in `readme_refresh` with `advisory: true`, an attempt date and
errors. They never replace catalog guidance or become executable commands.
Refresh currently reads the default branch and is not evidence about a supplied
pin. Ordinary catalog browsing and selection work offline.

## Review model

The current catalog has 15 resources and two dossiers. The other 13 are shown
as **unreviewed**. Client compatibility is catalog information, not a statement
that every listed host version was tested. A skills-only installation may omit
features provided by the upstream plugin or hooks.

Eight binary gates cover `license`, `data_egress`, `pinning`, `uninstall`,
`collisions`, `auto_update`, `maintenance_ownership` and `measurability`.
Four scores cover `automation_depth`, `token_efficiency`,
`integration_coverage` and `trust_signals`.

| Review status | Meaning |
| --- | --- |
| `clear` | All binary gates pass in the recorded review; not a safety certificate |
| `open_questions` | At least one unknown, with no failed binary gate |
| `known_risks` | At least one failed binary gate, with its evidence and resolution |
| `unreviewed` | No dossier for this resource |

A failed or unknown gate must describe how to resolve it. Each dossier includes
its source, install method, namespace, lifecycle and an `intersection_map` of
what it duplicates, replaces, conflicts with or complements in our accelerators.
The selector and site builder validate identity and recompute status before
using a dossier. Invalid or mismatched evidence is an error; a valid negative
review remains a record the team may choose to act on.

Popularity is discovery information, not security evidence. No claim of star
manipulation follows from a large counter. Automated isolation, measured token
savings and residue-free removal of arbitrary third-party tools are not
implemented guarantees of Kit 3.

## Verification

```bash
python3 scripts/validate_registry.py --check
python3 -m unittest tests.test_registry tests.test_open_source_kit tests.test_kit_fetcher tests.test_kit3 tests.test_kit3_catalog
bash -n kit3
shellcheck -S error kit3
./kit3 build --output output/kit3-site
```

Tests use temporary directories and process mocks for the native boundary; the
normal suite does not download or install third-party packages. A separate
manual smoke test can use an owned local Git repository to check add, list,
content update and removal through the pinned manager without installing an
upstream skill into a working project.

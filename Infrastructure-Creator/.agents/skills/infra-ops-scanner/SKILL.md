---
name: infra-ops-scanner
description: Detect a PHP target's containers, orchestration, CI/CD, IaC, deployment tooling, and deployment-target hints from real evidence, and flag destructive-command risks. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan the infra", "detect CI/CD", "how is this PHP app deployed", "what containers/orchestration does this use", "infra-ops-scanner".
phase: discovery
flow-next: profile-synthesizer
flow-alternatives: [profile-synthesizer]
related: [stack-scanner, architecture-scanner, integration-scanner, security-compliance-scanner, conventions-scanner, infra-scan]
---

# Infra Ops Scanner

## Overview

Read-only reconnaissance of how a PHP target is containerized, orchestrated, built in CI/CD, provisioned via IaC, and deployed. The goal is an evidence-backed operational picture that `profile-synthesizer` can turn into accurate generated skills, plus an explicit inventory of destructive commands so `hook-forge` can guard them.

The target project path is a **required** argument; never assume the current working directory is the target. Operate strictly read-only within it, under the contract's secrets rule.

## Outputs (MANDATORY)

Per run: exactly one report `tasks/TASK-{NNN}/infra-ops-scanner-findings.md` and exactly one evidence ledger `tasks/TASK-{NNN}/infra-ops-scanner-evidence.json`, both shaped by `stack-scanner/references/scan-evidence-contract.md` - read it first. Never write into the target.

## Process

1. **Detect containerization as a signal class, never from one filename** (contract, containerization checklist). A `Dockerfile` is one member; a compose file, a container-owning environment manager such as `.ddev/config.yaml`, `.lando.yml`, or `.devcontainer/`, and an image-pinning hosting manifest are equal members. Any present member proves containerization - name which one. From whichever file declares them, cite the PHP runtime (`FROM php:8.2-fpm` or a `php_version:` key), enabled extensions (`docker-php-ext-install`, `pecl install`, extension config), the service topology (web server, database, cache/queue), and build/Composer install steps. Report "no containerization" only after the whole class was searched, and say so.
2. **Detect orchestration.** Look for Kubernetes manifests (`*.yaml` with `kind:` Deployment/Service/Ingress, a `k8s/` or `deploy/` dir) and Helm charts (`Chart.yaml`, `values.yaml`, `templates/`). Cite the concrete files.
3. **Detect CI/CD and compile command definitions.** Look for `.github/workflows/*.yml`, `.gitlab-ci.yml`, `bitbucket-pipelines.yml`, `Jenkinsfile`, `azure-pipelines.yml`. Record the exact PHP-relevant stage command, working directory, prerequisites/services, environment class, and bounded anchor; resolve referenced Composer/npm scripts through the sibling `tasks/TASK-{NNN}/stack-scanner-findings.md`, and when it is absent record the command verbatim, mark the resolution `unknown`, and log the gap. Classify each command as non-mutating, workspace-mutating, network-capable, or external-side-effect-capable rather than trusting job/step names.
4. **Detect IaC.** Look for Terraform (`*.tf`, `*.tfvars`, `.terraform/`), Ansible (`playbook*.yml`, `roles/`, `inventory`), and Pulumi (`Pulumi.yaml`, `__main__.php`/language runtime). Note providers/resources only from visible files.
5. **Detect PHP deployment tooling.** Deployer (`deploy.php` with `Deployer\` usage), Laravel Envoy (`Envoy.blade.php`), Capistrano (`Capfile`, `config/deploy.rb`), and Forge/Ploi hints (deploy scripts, `.forge`/provider comments).
6. **Detect deployment-target hints.** Serverless/Bref (`serverless.yml`, `bref/bref` in composer), Platform.sh (`.platform.app.yaml`, `.platform/`), Heroku (`Procfile`, `app.json`), and generic PaaS config.
7. **Flag destructive-command risks across every command declaration site in the contract's checklist** - CI steps, deploy/release scripts, environment and container hooks (`.ddev/config.yaml` `hooks:` and custom commands, compose entrypoints), Makefile targets, cron/scheduler entries, and provider lifecycle hooks. Record with bounded anchors any presence of `kubectl` (apply/delete), `terraform apply`/`destroy`, `helm upgrade`/`uninstall`, schema/data writes (`doctrine:schema:update --force`, `doctrine:database:drop`, `migrate:fresh`, `migrate --force`, `db:wipe`), working-tree or history rewrites (`git reset --hard`, force push), `docker system prune`, formatter/fixer writes, or credential-backed provider commands - so later contracts can deny them by default. "None detected" is reportable only after the whole checklist was searched, and the report names the sites searched.
8. **Map operational path authority and adjacency.** Identify which deployment/config paths are required-existing versus explicitly creatable/generated, and map release, migration, async/provider, security, and domain owners for each material operation. Capture representative requests that should route to each primary/deferred owner.
9. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Report Structure

Follow the `infra-ops-scanner` report template in appendix A of `stack-scanner/references/scan-evidence-contract.md`. Every factual line carries its confidence tag and its evidence id.

## Guardrails

- MUST cite a real file path (and line where practical) for every finding, and MUST emit both artifacts with contract-shaped evidence records (target-relative path, `sha256:` fingerprint, supported claims).
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST detect containerization and destructive commands by searching the whole contract checklist, naming what was searched before reporting absence.
- MUST record destructive commands verbatim with location so `hook-forge` can guard them; MUST NOT execute them.
- MUST classify resolved commands by effects; a command named `lint`, `test`, or `check` is not automatically non-mutating.
- MUST NOT claim deploy, rollback, restart, balancing, or provisioning ownership unless target evidence proves that operation and its authority.
- MUST report absent tooling as `N/A - not configured` rather than assuming a default.
- MUST NOT deep-dive framework identity, integrations, security, or conventions - those belong to their own scanners.

## Final Output

Return both artifact paths (report and evidence ledger), the container/orchestration/CI-CD summary, the deployment tooling and target hints, the destructive-command risk list, and a one-line confidence summary. Suggest `profile-synthesizer` as the next step.

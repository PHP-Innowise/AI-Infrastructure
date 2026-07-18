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

The target project path is a **required** argument (e.g. "scan the infra of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, never read its `.env`/secrets.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/infra-ops-scanner-findings.md`. Never write into the target.

## Process

1. **Detect containers.** Look for `Dockerfile`(s), `docker-compose.yml`/`compose.yaml`, and `.dockerignore`. From the `Dockerfile`, cite the PHP base image (e.g. `FROM php:8.2-fpm`) and enabled extensions (`docker-php-ext-install`, `pecl install`), plus any multi-stage build and Composer install steps.
2. **Detect orchestration.** Look for Kubernetes manifests (`*.yaml` with `kind:` Deployment/Service/Ingress, a `k8s/` or `deploy/` dir) and Helm charts (`Chart.yaml`, `values.yaml`, `templates/`). Cite the concrete files.
3. **Detect CI/CD.** Look for `.github/workflows/*.yml`, `.gitlab-ci.yml`, `bitbucket-pipelines.yml`, `Jenkinsfile`, `azure-pipelines.yml`. Summarize the PHP-relevant stages (composer install, test, lint/static analysis, build, deploy) with file+line citations.
4. **Detect IaC.** Look for Terraform (`*.tf`, `*.tfvars`, `.terraform/`), Ansible (`playbook*.yml`, `roles/`, `inventory`), and Pulumi (`Pulumi.yaml`, `__main__.php`/language runtime). Note providers/resources only from visible files.
5. **Detect PHP deployment tooling.** Deployer (`deploy.php` with `Deployer\` usage), Laravel Envoy (`Envoy.blade.php`), Capistrano (`Capfile`, `config/deploy.rb`), and Forge/Ploi hints (deploy scripts, `.forge`/provider comments).
6. **Detect deployment-target hints.** Serverless/Bref (`serverless.yml`, `bref/bref` in composer), Platform.sh (`.platform.app.yaml`, `.platform/`), Heroku (`Procfile`, `app.json`), and generic PaaS config.
7. **Flag destructive-command risks.** Record any presence of `kubectl` (apply/delete), `terraform apply`/`destroy`, `helm upgrade`/`uninstall`, `php artisan migrate:fresh`/`migrate --force`/`db:wipe`, `docker system prune`, or force-push deploy steps - with file+line - so `hook-forge` can guard them.
8. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Output Template

```markdown
# Infra Ops Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Containers
- Dockerfile: base image [php:X-fpm], extensions [...] (confirmed - Dockerfile:L#)
- Compose / .dockerignore: [present/absent] (confirmed - path)

## Orchestration
- Kubernetes / Helm: [manifests/charts or "N/A - not configured"] (confirmed - path)

## CI/CD
- [provider]: stages [install/test/lint/build/deploy] (confirmed - path:L#)

## Infrastructure as Code
- Terraform / Ansible / Pulumi: [resources or "N/A - not configured"] (confirmed - path)

## Deployment Tooling
- [Deployer/Envoy/Capistrano/Forge/Ploi or "N/A"] (confirmed/inferred - path)

## Deployment Target Hints
- [Bref/Platform.sh/Heroku/PaaS or "N/A"] (confirmed/inferred - path)

## Destructive-Command Risks (for hook-forge)
- [command: file:L# + risk] for each; "None detected" if absent

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST record destructive commands verbatim with location so `hook-forge` can guard them; MUST NOT execute them.
- MUST report absent tooling as `N/A - not configured` rather than assuming a default.
- MUST NOT deep-dive framework identity, integrations, security, or conventions - those belong to their own scanners.

## Final Output

Return the findings file path, the container/orchestration/CI-CD summary, the deployment tooling and target hints, the destructive-command risk list, and a one-line confidence summary. Suggest `profile-synthesizer` as the next step.

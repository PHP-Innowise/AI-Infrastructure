---
name: architecture-implementer
description: Scaffold and wire approved Symfony layered architecture into controllers, services, repositories, DTOs/forms/voters/messages, Doctrine entities, and DI-ready classes.
phase: execution
flow-next: coder
flow-alternatives: [test-generator, code-reviewer, verify]
---

# Symfony Architecture Implementer

## Goal

Turn an approved design into a compiling Symfony skeleton without hiding business logic in the wrong layer.

## What To Create

Create only what the architecture requires:

- Controller classes with routes and injected services.
- Service/use-case classes with typed constructor dependencies and TODO-free method signatures when possible.
- Repository methods/classes for Doctrine reads/writes.
- Request/response DTOs, Forms, Validator constraints, or serializer models.
- Voters or security attributes/config placeholders.
- Messenger messages/handlers that delegate to services.
- Console commands that validate input and delegate to services.
- Doctrine entities/migrations only when the schema decision is approved.
- Tests or test skeletons when they clarify contracts.

## Rules

- Do not implement full business behavior unless the task asks for it; leave clear method contracts for `/coder`.
- Do not place Doctrine queries in controllers or services.
- Do not make services return Symfony `Response` objects.
- Do not create interfaces for every class by default.
- Wire via constructor injection and Symfony autowiring conventions.
- Prefer existing namespaces and project patterns over invented module layouts.

## Verification

Run when available:

```bash
composer dump-autoload
php bin/console lint:container
php bin/console debug:router
vendor/bin/phpunit --filter <new contract>
```

Report missing tools as `N/A - tooling not configured`.

Compare the resulting wiring with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Remove pass-through layers and interfaces that do not own behavior or represent a real boundary.

## Output

Include:

- Skeletons created.
- Controller/service/repository map.
- DI/autowiring notes.
- Remaining business logic for `/coder`.
- Verification evidence.

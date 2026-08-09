# PracticePerfect operator entrypoints.
#
# Every target runs inside the Compose containers. Nothing here requires PHP,
# Composer, Node or PostgreSQL on the host — Docker is the only prerequisite.

COMPOSE ?= docker compose
PHP     := $(COMPOSE) exec -T php
CONSOLE := $(PHP) php bin/console

.DEFAULT_GOAL := help
.PHONY: help setup env-reset build up down destroy logs ps shell install migrate migration seed reset test lint smoke health

help: ## List available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

setup: ## Create .env from the template if it does not exist
	@test -f .env || (cp env.example .env && echo "created .env from env.example")

env-reset: ## Overwrite .env from the template, discarding any local edits
	@cp env.example .env && echo "reset .env from env.example"

build: setup ## Build the application image
	$(COMPOSE) build

# The worker cannot become healthy before its transport table exists, and that
# table is created by a migration — so the stack comes up, migrates, then the
# worker is waited on. Plain `docker compose up -d` also works: the worker waits
# for the table instead of crash-looping, and goes healthy once you migrate.
up: setup ## Start the whole stack, migrate, and wait for every service to be healthy
	$(COMPOSE) up -d --wait db php web mail
	@$(MAKE) --no-print-directory migrate
	$(COMPOSE) up -d --wait worker
	@$(MAKE) --no-print-directory health

down: ## Stop the stack, keeping data
	$(COMPOSE) down

destroy: ## Stop the stack and delete all volumes, including the database
	$(COMPOSE) down -v

ps: ## Show service status
	$(COMPOSE) ps

logs: ## Follow logs for all services
	$(COMPOSE) logs -f

shell: ## Open a shell in the php container
	$(COMPOSE) exec php sh

install: ## Install PHP dependencies inside the container
	$(PHP) composer install --no-interaction

# Migrations run as the owner role. The application role holds no DDL, which is
# what keeps Row-Level Security meaningful, so DATABASE_URL is swapped for the
# owner DSN for the duration of the command only.
migrate: ## Apply Doctrine migrations (runs as the owner role)
	$(PHP) sh -c 'DATABASE_URL="$$MIGRATION_DATABASE_URL" php bin/console doctrine:migrations:migrate --no-interaction --allow-no-migration'

migration: ## Generate a new migration from mapping changes (runs as the owner role)
	$(PHP) sh -c 'DATABASE_URL="$$MIGRATION_DATABASE_URL" php bin/console doctrine:migrations:diff --no-interaction'

seed: ## Load fixtures: the four MVP roles a reviewer can log in as
	$(CONSOLE) doctrine:fixtures:load --no-interaction

reset: destroy up migrate seed ## Rebuild the database from nothing and reseed

# The test database is created and migrated by the owner role, then granted to
# the application role: PostgreSQL default privileges are per-database, so the
# initdb bootstrap does not reach it.
test-db: ## Create, grant and migrate the test database
	$(PHP) sh -c 'APP_ENV=test DATABASE_URL="$$MIGRATION_DATABASE_URL" php bin/console doctrine:database:create --if-not-exists'
	$(PHP) sh -c 'sh docker/postgres/grant-roles.sh "$${POSTGRES_DB}_test"'
	$(PHP) sh -c 'APP_ENV=test DATABASE_URL="$$MIGRATION_DATABASE_URL" php bin/console doctrine:migrations:migrate --no-interaction --allow-no-migration'

test: test-db ## Run the test suite inside the container
	$(PHP) php vendor/bin/phpunit

lint: ## Run all linters and static checks
	$(CONSOLE) lint:container
	$(CONSOLE) lint:yaml config
	$(CONSOLE) lint:twig templates
	$(PHP) composer validate --strict
	$(PHP) php vendor/bin/phpstan analyse --no-progress

smoke: ## Prove the running stack answers over HTTP
	@curl -fsS http://localhost:$${HTTP_PORT:-8080}/health && echo
	@echo "smoke: OK"

health: ## Show the health state of every service
	@$(COMPOSE) ps --format 'table {{.Service}}\t{{.Status}}'

# PracticePerfect operator entrypoints, Laravel edition.
#
# Every target runs inside the Compose containers. Nothing here requires PHP,
# Composer, Node or PostgreSQL on the host — Docker is the only prerequisite.

COMPOSE ?= docker compose
PHP     := $(COMPOSE) exec -T php
ARTISAN := $(PHP) php artisan

.DEFAULT_GOAL := help
.PHONY: help setup env-reset build up down destroy logs ps shell install migrate seed reset test test-db lint smoke health assets

help: ## List available targets
	@grep -hE '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

setup: ## Create .env from the template if it does not exist
	@test -f .env || (cp env.example .env && echo "created .env from env.example")

env-reset: ## Overwrite .env from the template, discarding local edits
	@cp env.example .env && echo "reset .env from env.example"

build: setup ## Build the application image (includes the Vite asset stage)
	$(COMPOSE) build

# The worker cannot become healthy before the jobs table exists, and that table
# is created by a migration — so the stack comes up, migrates, then the worker
# is waited on. Plain `docker compose up -d` also works: the worker waits for
# the table instead of crash-looping.
# Order matters, and not in the obvious way. `php`'s healthcheck is the fpm
# ping — it passes without the application being bootable. `web`'s healthcheck
# actually executes Laravel, so it fails until an APP_KEY exists and the schema
# is migrated. Install and migrate therefore run BEFORE web and worker are
# waited on, not after.
up: setup ## Start the stack, install, migrate, and wait for every service
	$(COMPOSE) up -d --wait db php mail
	@$(MAKE) --no-print-directory install
	@$(MAKE) --no-print-directory migrate
	$(COMPOSE) up -d --wait web worker
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

install: ## Install PHP dependencies and ensure an application key exists
	$(PHP) composer install --no-interaction
	@$(PHP) sh -c 'grep -q "^APP_KEY=base64:" .env || php artisan key:generate --force'

# Migrations run as the owner role. The application role holds no DDL, which is
# what keeps Row-Level Security meaningful, so the owner credentials are
# substituted for the duration of the command only.
migrate: ## Apply migrations (runs as the owner role)
	$(PHP) sh -c 'DB_USERNAME="$$DB_OWNER_USERNAME" DB_PASSWORD="$$DB_OWNER_PASSWORD" php artisan migrate --force'

# Seeding also runs as the owner: under Row-Level Security the application role
# would silently delete nothing from a trainer-scoped table when no tenant is
# set, so a re-seed would collide with a unique constraint instead of replacing.
seed: ## Load seeders: a login for each of the four MVP roles
	$(PHP) sh -c 'DB_USERNAME="$$DB_OWNER_USERNAME" DB_PASSWORD="$$DB_OWNER_PASSWORD" php artisan db:seed --force'

reset: destroy up seed ## Rebuild the database from nothing and reseed

assets: ## Rebuild the Vite bundle inside the image
	$(COMPOSE) build php

# The suite runs against PostgreSQL, not sqlite: Row-Level Security is the
# tenancy mechanism and sqlite has none, so a sqlite suite would report every
# tenancy test as passing while proving nothing. The test database is created
# and migrated by the owner, then granted to the application role — PostgreSQL
# default privileges are per-database, so the initdb bootstrap does not reach it.
test-db: ## Create, grant and migrate the test database
	$(PHP) sh -c 'PGPASSWORD="$$DB_OWNER_PASSWORD" psql -h "$$DB_HOST" -U "$$DB_OWNER_USERNAME" -d postgres -tc "SELECT 1 FROM pg_database WHERE datname = '"'"'practiceperfect_test'"'"'" | grep -q 1 || PGPASSWORD="$$DB_OWNER_PASSWORD" createdb -h "$$DB_HOST" -U "$$DB_OWNER_USERNAME" practiceperfect_test'
	$(PHP) sh docker/postgres/grant-roles.sh practiceperfect_test
	$(PHP) sh -c 'DB_DATABASE=practiceperfect_test DB_USERNAME="$$DB_OWNER_USERNAME" DB_PASSWORD="$$DB_OWNER_PASSWORD" php artisan migrate --force'

test: test-db ## Run the test suite inside the container
	$(PHP) php artisan test

lint: ## Run linters and static analysis
	$(PHP) ./vendor/bin/pint --test
	$(PHP) php artisan about --only=environment

smoke: ## Prove the running stack answers over HTTP
	@curl -fsS http://localhost:$${HTTP_PORT:-8091}/health && echo
	@echo "smoke: OK"

health: ## Show the health state of every service
	@$(COMPOSE) ps --format 'table {{.Service}}\t{{.Status}}'

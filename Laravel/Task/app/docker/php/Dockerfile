# PracticePerfect PHP image, Laravel edition.
#
# PHP 8.4 per specs/architect-architecture.md "Stack": intdiv for the integer
# money path, backed enums, readonly classes.

# --- Asset build stage -------------------------------------------------------
# Vite and Tailwind run here, never on the host. The Compose-only constraint
# forbids a host Node toolchain, and this keeps the runtime image free of one.
FROM node:22-alpine AS assets

WORKDIR /build
COPY package.json package-lock.json* ./
RUN npm ci --no-audit --no-fund 2>/dev/null || npm install --no-audit --no-fund
COPY vite.config.js ./
COPY resources ./resources
RUN npm run build

# --- Runtime -----------------------------------------------------------------
FROM php:8.4-fpm-alpine

ARG UID=1000
ARG GID=1000

RUN set -eux; \
    apk add --no-cache \
        acl \
        fcgi \
        git \
        icu-libs \
        libpq \
        libzip \
        postgresql-client \
        procps \
        su-exec \
        unzip; \
    apk add --no-cache --virtual .build-deps \
        $PHPIZE_DEPS \
        icu-dev \
        libpq-dev \
        libzip-dev \
        linux-headers; \
    docker-php-ext-install -j"$(nproc)" \
        bcmath \
        intl \
        opcache \
        pdo_pgsql \
        zip; \
    pecl install apcu; \
    docker-php-ext-enable apcu; \
    apk del --no-network .build-deps; \
    rm -rf /tmp/pear

COPY --from=composer:2 /usr/bin/composer /usr/local/bin/composer

# Run as the invoking host user so files written inside the container stay
# editable outside it.
RUN set -eux; \
    if ! getent group "${GID}" >/dev/null; then addgroup -g "${GID}" app; fi; \
    if ! getent passwd "${UID}" >/dev/null; then \
        adduser -u "${UID}" -G "$(getent group "${GID}" | cut -d: -f1)" -s /bin/sh -D app; \
    fi

COPY docker/php/php.ini /usr/local/etc/php/conf.d/zz-app.ini
COPY docker/php/fpm-pool.conf /usr/local/etc/php-fpm.d/zz-app.conf
COPY docker/php/docker-entrypoint.sh /usr/local/bin/docker-entrypoint
COPY docker/php/healthcheck.sh /usr/local/bin/php-healthcheck

RUN chmod +x /usr/local/bin/docker-entrypoint /usr/local/bin/php-healthcheck

# Compiled assets from the build stage. The bind mount in compose shadows /app
# during development; `make assets` re-runs the build when resources change.
COPY --from=assets /build/public/build /opt/assets/build

ENV COMPOSER_ALLOW_SUPERUSER=1 \
    COMPOSER_HOME=/tmp/composer \
    COMPOSER_MEMORY_LIMIT=-1 \
    APP_UID=${UID} \
    APP_GID=${GID}

WORKDIR /app

ENTRYPOINT ["docker-entrypoint"]
CMD ["php-fpm"]

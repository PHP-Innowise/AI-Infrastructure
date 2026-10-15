#!/bin/sh
# php-fpm liveness: ask the pool itself, not the operating system, whether it
# can still accept and answer a FastCGI request.
set -eu

SCRIPT_NAME=/ping \
SCRIPT_FILENAME=/ping \
REQUEST_METHOD=GET \
cgi-fcgi -bind -connect 127.0.0.1:9000 2>/dev/null | grep -q pong

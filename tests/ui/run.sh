#!/bin/sh
# Runs inside the Playwright container (see compose.test.yaml). Arguments go to `playwright test`.
set -eu
cd /work
# Installs only when package-lock.json has changed since the last run.
stamp=node_modules/.installed-for
if [ ! -f "$stamp" ] || ! cmp -s package-lock.json "$stamp"; then
	npm ci --no-audit --no-fund
	cp package-lock.json "$stamp"
fi
exec npx playwright test "$@"

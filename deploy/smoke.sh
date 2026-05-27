#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1300 smoke probe.
#
# Verifies every service in docker-compose.yml is alive and
# answering its documented health endpoint. Exits 0 on
# all-green, 1 on the first failure (with a one-line reason
# printed before exit).
#
# Run after `docker compose up -d`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

COMPOSE="${COMPOSE:-docker compose}"

# Pretty-print helpers.
green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*" >&2; }

fail=0

check() {
  local name="$1"
  local cmd="$2"
  printf '  %-22s ' "${name}..."
  if bash -c "${cmd}" >/dev/null 2>&1; then
    green "ok"
  else
    red "FAIL"
    red "    command: ${cmd}"
    fail=$((fail + 1))
  fi
}

echo "InvoiceKit smoke probe"
echo

check "postgres"          "${COMPOSE} exec -T postgres pg_isready -U invoicekit -d invoicekit"
check "managed-api"       "curl -fsS http://127.0.0.1:8080/health"
check "validator-kosit"   "${COMPOSE} exec -T validator-kosit wget -qO- http://localhost:8080/health"
check "validator-phive"   "${COMPOSE} exec -T validator-phive wget -qO- http://localhost:8080/health"
check "validator-saxon"   "${COMPOSE} exec -T validator-saxon wget -qO- http://localhost:8080/health"
check "validator-verapdf" "${COMPOSE} exec -T validator-verapdf wget -qO- http://localhost:8080/health"
check "validator-phase4"  "${COMPOSE} exec -T validator-phase4 wget -qO- --post-data='{\"method\":\"health\"}' --header='Content-Type: application/json' http://localhost:8090/"
check "archive-minio"     "${COMPOSE} exec -T archive-minio curl -fsS http://localhost:9000/minio/health/ready"
check "signer-agent"      "${COMPOSE} exec -T signer-agent test -S /run/invoicekit/signer.sock"

echo
if [[ ${fail} -eq 0 ]]; then
  green "all services healthy"
  exit 0
fi
red "${fail} service(s) failed"
exit 1

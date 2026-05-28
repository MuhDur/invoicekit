# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
# T-035: validator UI static-host image — validate.invoicekit.org.

FROM oven/bun:1.1 AS builder
WORKDIR /app
COPY apps/validator-ui/package.json apps/validator-ui/bun.lock* ./
RUN bun install --frozen-lockfile || bun install
COPY apps/validator-ui/ ./
ENV VITE_REFERENCE_VALIDATOR_URL=https://reference.validate.invoicekit.org
RUN bun run build

FROM nginx:1.27-alpine AS runtime
COPY --from=builder /app/dist /usr/share/nginx/html
COPY apps/validator-ui/nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/ >/dev/null 2>&1 || exit 1

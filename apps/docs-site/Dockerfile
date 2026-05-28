# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
# T-113: docs site static-host image — docs.invoicekit.org.

FROM python:3.12-slim AS generator
WORKDIR /repo
COPY tools/docs-generate /repo/tools/docs-generate
COPY crates/rulepack/data /repo/crates/rulepack/data
COPY crates/ /repo/crates/
COPY docs/operators /repo/docs/operators
COPY apps/docs-site/pages/index.mdx apps/docs-site/pages/_meta.json /repo/apps/docs-site/pages/
COPY apps/docs-site/pages/rules/index.mdx /repo/apps/docs-site/pages/rules/
COPY apps/docs-site/pages/countries/index.mdx /repo/apps/docs-site/pages/countries/
COPY apps/docs-site/pages/operators/index.mdx /repo/apps/docs-site/pages/operators/
RUN python3 tools/docs-generate/generate.py

FROM oven/bun:1.1 AS builder
WORKDIR /app
COPY apps/docs-site/package.json apps/docs-site/bun.lock* ./
RUN bun install --frozen-lockfile || bun install
COPY apps/docs-site/ ./
COPY --from=generator /repo/apps/docs-site/pages ./pages
RUN bun run build

FROM nginx:1.27-alpine AS runtime
COPY --from=builder /app/out /usr/share/nginx/html
COPY apps/docs-site/nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/ >/dev/null 2>&1 || exit 1

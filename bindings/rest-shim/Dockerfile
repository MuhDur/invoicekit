# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

FROM rust:1.83-bookworm AS builder

WORKDIR /workspace
COPY . .
RUN cargo build --locked --release -p invoicekit-binding-rest-shim && \
    cp target/release/invoicekit-binding-rest-shim /tmp/invoicekit-rest-shim

FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /tmp/invoicekit-rest-shim /usr/local/bin/invoicekit-rest-shim

ENV INVOICEKIT_REST_BIND=0.0.0.0:8081
ENV INVOICEKIT_REST_HEALTH_URL=http://127.0.0.1:8081/healthz

EXPOSE 8081

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/invoicekit-rest-shim", "--healthcheck"]

USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/invoicekit-rest-shim"]

# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-052 sidecar image: validator-verapdf.
#
# Mirrors the validator-kosit / validator-phive / validator-saxon
# layout exactly. The same `services/validator-common` JAR is built
# with the `verapdf` Maven profile, which pulls in
# `org.verapdf:verapdf-library` and selects the `jvm:verapdf`
# BackendConfig at runtime via INVOICEKIT_VALIDATOR_BACKEND.

FROM maven:3.9.11-eclipse-temurin-21 AS build
WORKDIR /workspace
COPY services/validator-common/pom.xml ./pom.xml
COPY services/validator-common/src ./src
RUN mvn -q -Pverapdf -DskipTests package

FROM eclipse-temurin:21-jre
WORKDIR /app
ENV INVOICEKIT_VALIDATOR_BACKEND=jvm:verapdf
ENV PORT=8080
COPY --from=build /workspace/target/invoicekit-validator-sidecar-0.1.0.jar ./invoicekit-validator-sidecar.jar
COPY --from=build /workspace/target/dependency ./lib
EXPOSE 8080
ENTRYPOINT ["java", "-cp", "/app/invoicekit-validator-sidecar.jar:/app/lib/*", "dev.invoicekit.validator.ValidatorSidecar"]

# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

FROM maven:3.9.11-eclipse-temurin-21 AS build
WORKDIR /workspace
ARG KOSIT_XRECHNUNG_CONFIG_URL="https://github.com/itplr-kosit/validator-configuration-xrechnung/releases/download/v2026-01-31/xrechnung-3.0.2-validator-configuration-2026-01-31.zip"
ARG KOSIT_XRECHNUNG_CONFIG_SHA256="6a5a5911a421b25fbc423f62f93f894df7b236f5d73ca4f84bb222a945082704"
RUN curl -fsSL "${KOSIT_XRECHNUNG_CONFIG_URL}" -o ./kosit-xrechnung.zip \
  && echo "${KOSIT_XRECHNUNG_CONFIG_SHA256}  ./kosit-xrechnung.zip" | sha256sum -c -
COPY services/validator-common/pom.xml ./pom.xml
COPY services/validator-common/src ./src
RUN mvn -q -Pkosit -DskipTests package
RUN mkdir -p /workspace/kosit-xrechnung \
  && cd /workspace/kosit-xrechnung \
  && jar xf ../kosit-xrechnung.zip \
  && test -f scenarios.xml

FROM eclipse-temurin:21-jre
WORKDIR /app
ENV INVOICEKIT_VALIDATOR_BACKEND=jvm:kosit
ENV INVOICEKIT_VALIDATOR_KOSIT_SCENARIOS=/app/kosit-xrechnung/scenarios.xml
ENV PORT=8080
COPY --from=build /workspace/target/invoicekit-validator-sidecar-0.1.0.jar ./invoicekit-validator-sidecar.jar
COPY --from=build /workspace/target/dependency ./lib
COPY --from=build /workspace/kosit-xrechnung ./kosit-xrechnung
EXPOSE 8080
ENTRYPOINT ["java", "-cp", "/app/invoicekit-validator-sidecar.jar:/app/lib/*", "dev.invoicekit.validator.ValidatorSidecar"]

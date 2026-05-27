# syntax=docker/dockerfile:1.7
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

FROM maven:3.9.11-eclipse-temurin-21 AS build
WORKDIR /workspace
COPY services/validator-phase4/pom.xml ./pom.xml
COPY services/validator-phase4/src ./src
RUN mvn -q -DskipTests package

FROM eclipse-temurin:21-jre
WORKDIR /opt/phase4
ENV INVOICEKIT_PHASE4_PORT=8090 \
    PEPPOL_AP_SML_MODE=acceptance
COPY --from=build /workspace/target/phase4-server.jar ./phase4-server.jar
COPY --from=build /workspace/target/dependency ./lib
EXPOSE 8090
ENTRYPOINT ["java", "-cp", "/opt/phase4/phase4-server.jar:/opt/phase4/lib/*", "dev.invoicekit.phase4.Phase4Server"]

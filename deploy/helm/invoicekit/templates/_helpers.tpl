{{/*
  SPDX-License-Identifier: Apache-2.0
  Copyright 2026 The InvoiceKit Authors
*/}}

{{/* Chart fullname (release-prefixed). */}}
{{- define "invoicekit.fullname" -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{/* Standard labels stamped on every resource. */}}
{{- define "invoicekit.labels" -}}
app.kubernetes.io/name: {{ default .Chart.Name .Values.nameOverride }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- with .Values.commonLabels }}
{{ toYaml . }}
{{- end }}
{{- end -}}

{{/* Selector labels for a given component. */}}
{{- define "invoicekit.selectorLabels" -}}
app.kubernetes.io/name: {{ default .Chart.Name .Values.nameOverride }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{/* Build a fully-qualified image reference for a component. */}}
{{- define "invoicekit.image" -}}
{{- $registry := .Values.global.imageRegistry -}}
{{- $repo := .image.repository -}}
{{- $tag := default .Values.global.imageTag .image.tag -}}
{{- if contains "/" $repo -}}
{{ printf "%s:%s" $repo $tag }}
{{- else -}}
{{ printf "%s/%s:%s" $registry $repo $tag }}
{{- end -}}
{{- end -}}

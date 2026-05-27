# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1401 demo URL conf — three endpoints mirroring the FastAPI demo.

from __future__ import annotations

from django.urls import path

from . import views


urlpatterns = [
    path("", views.index),
    path("healthz", views.healthz),
    path("canonicalize/<str:fixture_name>", views.canonicalize_endpoint),
]

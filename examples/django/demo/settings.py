# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# T-1401 demo settings — the minimum that Django needs to serve a
# single JSON endpoint backed by the InvoiceKit Rust engine.

from __future__ import annotations

import os
from pathlib import Path


BASE_DIR = Path(__file__).resolve().parent.parent

# The demo is intentionally NOT meant for production. Override
# DJANGO_SECRET_KEY before deploying anywhere public.
SECRET_KEY = os.environ.get(
    "DJANGO_SECRET_KEY",
    "demo-only-do-not-use-in-production-invoicekit-django",
)
DEBUG = os.environ.get("DJANGO_DEBUG", "1") == "1"
ALLOWED_HOSTS = ["*"]

INSTALLED_APPS = [
    "django.contrib.contenttypes",
    "django.contrib.auth",
]
MIDDLEWARE: list[str] = []
ROOT_URLCONF = "demo.urls"
TEMPLATES: list[dict[str, object]] = []
WSGI_APPLICATION = "demo.wsgi.application"

DATABASES = {
    "default": {
        "ENGINE": "django.db.backends.sqlite3",
        "NAME": ":memory:",
    }
}
DEFAULT_AUTO_FIELD = "django.db.models.BigAutoField"

USE_TZ = True

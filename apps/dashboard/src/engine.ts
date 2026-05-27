export type BillingState = "trial" | "active" | "past_due" | "suspended";

export type ActivityKind = "sent" | "validated" | "failed" | "archived";

export type AuditOutcome = "denied" | "failed" | "succeeded";

export type RecentErrorSeverity = "critical" | "info" | "warning";

export type RecentErrorSource = "gateway" | "retry" | "system" | "validator";

export type TransmissionState = "accepted" | "archived" | "failed" | "queued" | "rejected" | "sent" | "validating";

export interface RecentActivityItem {
  readonly id: string;
  readonly kind: ActivityKind;
  readonly documentId: string;
  readonly counterparty: string;
  readonly occurredAt: string;
  readonly summary: string;
  readonly traceId: string;
}

export interface DocumentsSentGauge {
  readonly used: number;
  readonly limit: number;
  readonly periodLabel: string;
}

export interface BillingBanner {
  readonly state: BillingState;
  readonly headline: string;
  readonly detail: string;
  readonly actionLabel?: string;
}

export interface TenantOverview {
  readonly tenantId: string;
  readonly tenantName: string;
  readonly documentsSent: DocumentsSentGauge;
  readonly billing: BillingBanner;
  readonly recentActivity: readonly RecentActivityItem[];
  readonly validationFailures: number;
  readonly generatedAt: string;
}

export interface UsageMonth {
  readonly month: string;
  readonly documentsSent: number;
  readonly documentsReceived: number;
  readonly partnerApCost: string;
}

export interface TenantUsage {
  readonly periodLabel: string;
  readonly totalSent: number;
  readonly totalReceived: number;
  readonly partnerApCostTotal: string;
  readonly currency: string;
  readonly months: readonly UsageMonth[];
}

export interface RecentError {
  readonly id: string;
  readonly occurredAt: string;
  readonly source: RecentErrorSource;
  readonly severity: RecentErrorSeverity;
  readonly documentId: string;
  readonly summary: string;
  readonly remediation: string;
  readonly traceId: string;
}

export interface RecentErrorPage {
  readonly items: readonly RecentError[];
  readonly pageInfo: TransmissionPageInfo;
}

export interface RecentErrorListParams {
  readonly cursor?: string;
  readonly limit?: number;
  readonly severity?: RecentErrorSeverity;
}

export interface AuditEvent {
  readonly id: string;
  readonly occurredAt: string;
  readonly actor: string;
  readonly action: string;
  readonly resourceType: string;
  readonly resourceId: string;
  readonly outcome: AuditOutcome;
  readonly traceId: string;
}

export interface AuditEventPage {
  readonly items: readonly AuditEvent[];
  readonly pageInfo: TransmissionPageInfo;
}

export interface AuditEventListParams {
  readonly actor?: string;
  readonly cursor?: string;
  readonly limit?: number;
  readonly outcome?: AuditOutcome;
}

export interface TransmissionSummary {
  readonly id: string;
  readonly documentId: string;
  readonly state: TransmissionState;
  readonly gateway: string;
  readonly recipient: string;
  readonly recipientCountry: string;
  readonly issueDate: string;
  readonly updatedAt: string;
  readonly amount: string;
  readonly currency: string;
  readonly receiptUrl?: string;
  readonly evidenceBundleUrl?: string;
}

export interface TransmissionPageInfo {
  readonly endCursor?: string;
  readonly hasNextPage: boolean;
  readonly limit: number;
}

export interface TransmissionPage {
  readonly items: readonly TransmissionSummary[];
  readonly pageInfo: TransmissionPageInfo;
}

export interface TransmissionListParams {
  readonly cursor?: string;
  readonly limit?: number;
}

export interface DashboardEngineClient {
  listAuditEvents(params?: AuditEventListParams): Promise<AuditEventPage>;
  listRecentErrors(params?: RecentErrorListParams): Promise<RecentErrorPage>;
  listTransmissions(params?: TransmissionListParams): Promise<TransmissionPage>;
  tenantUsage(): Promise<TenantUsage>;
  tenantOverview(): Promise<TenantOverview>;
}

export type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export interface EngineRpcClientOptions {
  readonly endpoint?: string;
  readonly fetcher?: FetchLike;
  readonly requestIdFactory?: () => string;
}

export class EngineRpcError extends Error {
  readonly code: number | undefined;
  readonly data: unknown;
  readonly status: number | undefined;

  constructor(message: string, options: { readonly code?: number; readonly data?: unknown; readonly status?: number } = {}) {
    super(message);
    this.name = "EngineRpcError";
    this.code = options.code;
    this.data = options.data;
    this.status = options.status;
  }
}

export function createHttpDashboardClient(options: EngineRpcClientOptions = {}): DashboardEngineClient {
  const endpoint = options.endpoint ?? dashboardEngineEndpoint();
  const fetcher = options.fetcher ?? globalThis.fetch?.bind(globalThis);
  const requestIdFactory = options.requestIdFactory ?? defaultRequestId;

  if (!fetcher) {
    throw new EngineRpcError("Fetch is not available for Engine ABI calls");
  }

  return {
    async listAuditEvents(params = {}) {
      return callEngineMethod({
        endpoint,
        fetcher,
        method: "engine.list_audit_events",
        params,
        parse: parseAuditEventPage,
        requestId: requestIdFactory()
      });
    },
    async listRecentErrors(params = {}) {
      return callEngineMethod({
        endpoint,
        fetcher,
        method: "engine.list_recent_errors",
        params,
        parse: parseRecentErrorPage,
        requestId: requestIdFactory()
      });
    },
    async listTransmissions(params = {}) {
      return callEngineMethod({
        endpoint,
        fetcher,
        method: "engine.list_transmissions",
        params,
        parse: parseTransmissionPage,
        requestId: requestIdFactory()
      });
    },
    async tenantUsage() {
      return callEngineMethod({
        endpoint,
        fetcher,
        method: "engine.tenant_usage",
        params: {},
        parse: parseTenantUsage,
        requestId: requestIdFactory()
      });
    },
    async tenantOverview() {
      return callEngineMethod({
        endpoint,
        fetcher,
        method: "engine.tenant_overview",
        params: {},
        parse: parseTenantOverview,
        requestId: requestIdFactory()
      });
    }
  };
}

export function usagePercent(gauge: DocumentsSentGauge): number {
  if (gauge.limit <= 0) {
    return 0;
  }

  return Math.min(100, Math.round((gauge.used / gauge.limit) * 100));
}

export function billingTone(state: BillingState): "neutral" | "good" | "warning" | "critical" {
  switch (state) {
    case "active":
      return "good";
    case "trial":
      return "neutral";
    case "past_due":
      return "warning";
    case "suspended":
      return "critical";
  }
}

interface EngineMethodCall<Result> {
  readonly endpoint: string;
  readonly fetcher: FetchLike;
  readonly method:
    | "engine.list_audit_events"
    | "engine.list_recent_errors"
    | "engine.list_transmissions"
    | "engine.tenant_overview"
    | "engine.tenant_usage";
  readonly params: unknown;
  readonly parse: (value: unknown) => Result;
  readonly requestId: string;
}

async function callEngineMethod<Result extends AuditEventPage | RecentErrorPage | TenantOverview | TenantUsage | TransmissionPage>({
  endpoint,
  fetcher,
  method,
  params,
  parse,
  requestId
}: EngineMethodCall<Result>): Promise<Result> {
  const response = await fetcher(endpoint, {
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: requestId,
      method,
      params
    }),
    credentials: "include",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/json"
    },
    method: "POST"
  });

  const payload = await readJson(response);

  if (!response.ok) {
    throw new EngineRpcError(`Engine ABI request failed with HTTP ${response.status}`, {
      data: payload,
      status: response.status
    });
  }

  const envelope = asRecord(payload, "Engine ABI response");
  const error = envelope.error;

  if (error !== undefined) {
    const errorRecord = asRecord(error, "Engine ABI error");
    const code = readOptionalNumber(errorRecord, "code", "Engine ABI error");
    throw new EngineRpcError(readString(errorRecord, "message", "Engine ABI error"), {
      data: errorRecord.data,
      ...(code !== undefined ? { code } : {})
    });
  }

  return parse(envelope.result);
}

async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch (error) {
    throw new EngineRpcError("Engine ABI returned invalid JSON", {
      data: error,
      status: response.status
    });
  }
}

function parseTenantOverview(value: unknown): TenantOverview {
  const record = asRecord(value, "tenant overview");

  return {
    tenantId: readString(record, "tenantId", "tenant overview"),
    tenantName: readString(record, "tenantName", "tenant overview"),
    documentsSent: parseDocumentsSent(readRequired(record, "documentsSent", "tenant overview")),
    billing: parseBilling(readRequired(record, "billing", "tenant overview")),
    recentActivity: readArray(record, "recentActivity", "tenant overview").map(parseRecentActivity),
    validationFailures: readNumber(record, "validationFailures", "tenant overview"),
    generatedAt: readString(record, "generatedAt", "tenant overview")
  };
}

function parseDocumentsSent(value: unknown): DocumentsSentGauge {
  const record = asRecord(value, "documents sent gauge");

  return {
    used: readNumber(record, "used", "documents sent gauge"),
    limit: readNumber(record, "limit", "documents sent gauge"),
    periodLabel: readString(record, "periodLabel", "documents sent gauge")
  };
}

function parseBilling(value: unknown): BillingBanner {
  const record = asRecord(value, "billing banner");
  const actionLabel = record.actionLabel;

  return {
    state: readBillingState(record, "state", "billing banner"),
    headline: readString(record, "headline", "billing banner"),
    detail: readString(record, "detail", "billing banner"),
    ...(typeof actionLabel === "string" ? { actionLabel } : {})
  };
}

function parseRecentActivity(value: unknown): RecentActivityItem {
  const record = asRecord(value, "recent activity item");

  return {
    id: readString(record, "id", "recent activity item"),
    kind: readActivityKind(record, "kind", "recent activity item"),
    documentId: readString(record, "documentId", "recent activity item"),
    counterparty: readString(record, "counterparty", "recent activity item"),
    occurredAt: readString(record, "occurredAt", "recent activity item"),
    summary: readString(record, "summary", "recent activity item"),
    traceId: readString(record, "traceId", "recent activity item")
  };
}

function parseAuditEventPage(value: unknown): AuditEventPage {
  const record = asRecord(value, "audit event page");

  return {
    items: readArray(record, "items", "audit event page").map(parseAuditEvent),
    pageInfo: parseTransmissionPageInfo(readRequired(record, "pageInfo", "audit event page"))
  };
}

function parseAuditEvent(value: unknown): AuditEvent {
  const record = asRecord(value, "audit event");

  return {
    id: readString(record, "id", "audit event"),
    occurredAt: readString(record, "occurredAt", "audit event"),
    actor: readString(record, "actor", "audit event"),
    action: readString(record, "action", "audit event"),
    resourceType: readString(record, "resourceType", "audit event"),
    resourceId: readString(record, "resourceId", "audit event"),
    outcome: readAuditOutcome(record, "outcome", "audit event"),
    traceId: readString(record, "traceId", "audit event")
  };
}

function parseRecentErrorPage(value: unknown): RecentErrorPage {
  const record = asRecord(value, "recent error page");

  return {
    items: readArray(record, "items", "recent error page").map(parseRecentError),
    pageInfo: parseTransmissionPageInfo(readRequired(record, "pageInfo", "recent error page"))
  };
}

function parseRecentError(value: unknown): RecentError {
  const record = asRecord(value, "recent error");

  return {
    id: readString(record, "id", "recent error"),
    occurredAt: readString(record, "occurredAt", "recent error"),
    source: readRecentErrorSource(record, "source", "recent error"),
    severity: readRecentErrorSeverity(record, "severity", "recent error"),
    documentId: readString(record, "documentId", "recent error"),
    summary: readString(record, "summary", "recent error"),
    remediation: readString(record, "remediation", "recent error"),
    traceId: readString(record, "traceId", "recent error")
  };
}

function parseTenantUsage(value: unknown): TenantUsage {
  const record = asRecord(value, "tenant usage");

  return {
    periodLabel: readString(record, "periodLabel", "tenant usage"),
    totalSent: readNumber(record, "totalSent", "tenant usage"),
    totalReceived: readNumber(record, "totalReceived", "tenant usage"),
    partnerApCostTotal: readString(record, "partnerApCostTotal", "tenant usage"),
    currency: readString(record, "currency", "tenant usage"),
    months: readArray(record, "months", "tenant usage").map(parseUsageMonth)
  };
}

function parseUsageMonth(value: unknown): UsageMonth {
  const record = asRecord(value, "usage month");

  return {
    month: readString(record, "month", "usage month"),
    documentsSent: readNumber(record, "documentsSent", "usage month"),
    documentsReceived: readNumber(record, "documentsReceived", "usage month"),
    partnerApCost: readString(record, "partnerApCost", "usage month")
  };
}

function parseTransmissionPage(value: unknown): TransmissionPage {
  const record = asRecord(value, "transmission page");

  return {
    items: readArray(record, "items", "transmission page").map(parseTransmissionSummary),
    pageInfo: parseTransmissionPageInfo(readRequired(record, "pageInfo", "transmission page"))
  };
}

function parseTransmissionPageInfo(value: unknown): TransmissionPageInfo {
  const record = asRecord(value, "transmission page info");
  const endCursor = record.endCursor;

  return {
    ...(typeof endCursor === "string" ? { endCursor } : {}),
    hasNextPage: readBoolean(record, "hasNextPage", "transmission page info"),
    limit: readNumber(record, "limit", "transmission page info")
  };
}

function parseTransmissionSummary(value: unknown): TransmissionSummary {
  const record = asRecord(value, "transmission summary");
  const receiptUrl = record.receiptUrl;
  const evidenceBundleUrl = record.evidenceBundleUrl;

  return {
    id: readString(record, "id", "transmission summary"),
    documentId: readString(record, "documentId", "transmission summary"),
    state: readTransmissionState(record, "state", "transmission summary"),
    gateway: readString(record, "gateway", "transmission summary"),
    recipient: readString(record, "recipient", "transmission summary"),
    recipientCountry: readString(record, "recipientCountry", "transmission summary"),
    issueDate: readString(record, "issueDate", "transmission summary"),
    updatedAt: readString(record, "updatedAt", "transmission summary"),
    amount: readString(record, "amount", "transmission summary"),
    currency: readString(record, "currency", "transmission summary"),
    ...(typeof receiptUrl === "string" ? { receiptUrl } : {}),
    ...(typeof evidenceBundleUrl === "string" ? { evidenceBundleUrl } : {})
  };
}

function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new EngineRpcError(`Invalid ${label}: expected object`, { data: value });
  }

  return value as Record<string, unknown>;
}

function readRequired(record: Record<string, unknown>, key: string, label: string): unknown {
  const value = record[key];

  if (value === undefined) {
    throw new EngineRpcError(`Invalid ${label}: missing ${key}`, { data: record });
  }

  return value;
}

function readString(record: Record<string, unknown>, key: string, label: string): string {
  const value = readRequired(record, key, label);

  if (typeof value !== "string") {
    throw new EngineRpcError(`Invalid ${label}: ${key} must be a string`, { data: record });
  }

  return value;
}

function readNumber(record: Record<string, unknown>, key: string, label: string): number {
  const value = readRequired(record, key, label);

  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new EngineRpcError(`Invalid ${label}: ${key} must be a finite number`, { data: record });
  }

  return value;
}

function readBoolean(record: Record<string, unknown>, key: string, label: string): boolean {
  const value = readRequired(record, key, label);

  if (typeof value !== "boolean") {
    throw new EngineRpcError(`Invalid ${label}: ${key} must be a boolean`, { data: record });
  }

  return value;
}

function readOptionalNumber(record: Record<string, unknown>, key: string, label: string): number | undefined {
  const value = record[key];

  if (value === undefined) {
    return undefined;
  }

  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new EngineRpcError(`Invalid ${label}: ${key} must be a finite number`, { data: record });
  }

  return value;
}

function readArray(record: Record<string, unknown>, key: string, label: string): readonly unknown[] {
  const value = readRequired(record, key, label);

  if (!Array.isArray(value)) {
    throw new EngineRpcError(`Invalid ${label}: ${key} must be an array`, { data: record });
  }

  return value;
}

function readBillingState(record: Record<string, unknown>, key: string, label: string): BillingState {
  const value = readString(record, key, label);

  if (value === "active" || value === "past_due" || value === "suspended" || value === "trial") {
    return value;
  }

  throw new EngineRpcError(`Invalid ${label}: unsupported billing state`, { data: record });
}

function readActivityKind(record: Record<string, unknown>, key: string, label: string): ActivityKind {
  const value = readString(record, key, label);

  if (value === "archived" || value === "failed" || value === "sent" || value === "validated") {
    return value;
  }

  throw new EngineRpcError(`Invalid ${label}: unsupported activity kind`, { data: record });
}

function readAuditOutcome(record: Record<string, unknown>, key: string, label: string): AuditOutcome {
  const value = readString(record, key, label);

  if (value === "denied" || value === "failed" || value === "succeeded") {
    return value;
  }

  throw new EngineRpcError(`Invalid ${label}: unsupported audit outcome`, { data: record });
}

function readRecentErrorSeverity(
  record: Record<string, unknown>,
  key: string,
  label: string
): RecentErrorSeverity {
  const value = readString(record, key, label);

  if (value === "critical" || value === "info" || value === "warning") {
    return value;
  }

  throw new EngineRpcError(`Invalid ${label}: unsupported error severity`, { data: record });
}

function readRecentErrorSource(record: Record<string, unknown>, key: string, label: string): RecentErrorSource {
  const value = readString(record, key, label);

  if (value === "gateway" || value === "retry" || value === "system" || value === "validator") {
    return value;
  }

  throw new EngineRpcError(`Invalid ${label}: unsupported error source`, { data: record });
}

function readTransmissionState(record: Record<string, unknown>, key: string, label: string): TransmissionState {
  const value = readString(record, key, label);

  if (
    value === "accepted" ||
    value === "archived" ||
    value === "failed" ||
    value === "queued" ||
    value === "rejected" ||
    value === "sent" ||
    value === "validating"
  ) {
    return value;
  }

  throw new EngineRpcError(`Invalid ${label}: unsupported transmission state`, { data: record });
}

function dashboardEngineEndpoint(): string {
  const env = (import.meta as ImportMeta & { readonly env?: Record<string, string | undefined> }).env;
  return env?.VITE_ENGINE_ABI_URL ?? "/engine";
}

function defaultRequestId(): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  return randomUuid ?? `dashboard_${Date.now()}`;
}

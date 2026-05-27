import type { ReactNode } from "react";

interface StatusPillProps {
  readonly children: ReactNode;
  readonly tone: "critical" | "good" | "neutral" | "warning";
}

export function StatusPill({ children, tone }: StatusPillProps) {
  return (
    <span className="status-pill" data-tone={tone}>
      {children}
    </span>
  );
}

import type { ReactNode } from "react";

interface MetricPanelProps {
  readonly foot: string;
  readonly icon: ReactNode;
  readonly label: string;
  readonly value: string;
  readonly children?: ReactNode;
}

export function MetricPanel({ children, foot, icon, label, value }: MetricPanelProps) {
  return (
    <section className="metric-panel" aria-label={label}>
      <div className="metric-heading">
        <span>{label}</span>
        {icon}
      </div>
      <div className="metric-value">{value}</div>
      {children}
      <div className="metric-foot">{foot}</div>
    </section>
  );
}

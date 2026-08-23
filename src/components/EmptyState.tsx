import type { ReactNode } from "react";
import { Icon, type IconName } from "./Icon";

interface EmptyStateProps {
  icon?: IconName;
  eyebrow?: string;
  title: string;
  description?: string;
  action?: ReactNode;
  compact?: boolean;
}

export function EmptyState({ icon = "folder", eyebrow, title, description, action, compact }: EmptyStateProps) {
  return (
    <section className={`generic-empty ${compact ? "is-compact" : ""}`}>
      <span className="generic-empty-icon"><Icon name={icon} /></span>
      {eyebrow && <p className="eyebrow">{eyebrow}</p>}
      <h2>{title}</h2>
      {description && <p>{description}</p>}
      {action}
    </section>
  );
}

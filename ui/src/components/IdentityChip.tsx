import type { Identity } from "../api/generated/model";
import styles from "./IdentityChip.module.css";

/**
 * Renders a caller identity (docs/02-domain-model.md): label when present,
 * otherwise agent model + harness. Full identity in the tooltip.
 */
export function IdentityChip({ identity }: { identity: Identity }) {
  const label = identity.label ?? `${identity.agent_model} · ${identity.harness}`;
  const full = `${identity.agent_model} on ${identity.harness} (session ${identity.session_id})`;
  return (
    <span className={styles.chip} title={full}>
      {label}
    </span>
  );
}

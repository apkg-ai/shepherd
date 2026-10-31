import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import type { Eligibility } from "../api/generated/model/eligibility";
import type { GateReason } from "../api/generated/model/gateReason";
import styles from "./EligibilityPanel.module.css";

/** Gate reason codes that reference an epic. */
const epicReasonCodes = new Set(["epic_proposal_required", "epic_block", "epic_prerequisite"]);

/** Gate reason codes that reference a task. */
const taskReasonCodes = new Set(["task_prerequisite", "claimed"]);

function resourceLink(projectId: string, reason: GateReason): string | undefined {
  if (epicReasonCodes.has(reason.code)) {
    return `/projects/${projectId}/epics/${reason.resource_id}`;
  }
  if (taskReasonCodes.has(reason.code)) {
    return `/projects/${projectId}/tasks/${reason.resource_id}`;
  }
  return undefined;
}

interface EligibilityPanelProps {
  eligibility: Eligibility;
  projectId: string;
}

export function EligibilityPanel({ eligibility, projectId }: EligibilityPanelProps) {
  const { t } = useTranslation();

  return (
    <section className={styles.panel} aria-label={t("eligibility.panel.title")}>
      <h3 className={styles.title}>{t("eligibility.panel.title")}</h3>
      <div className={styles.flags}>
        <Flag active={eligibility.can_plan} label={t("eligibility.panel.canPlan")} />
        <Flag active={eligibility.can_execute} label={t("eligibility.panel.canExecute")} />
        <Flag active={eligibility.can_review} label={t("eligibility.panel.canReview")} />
      </div>
      {eligibility.reasons.length > 0 ? (
        <ul className={styles.reasons} aria-label={t("eligibility.panel.blocked")}>
          {eligibility.reasons.map((reason, i) => {
            const link = resourceLink(projectId, reason);
            return (
              <li key={`${reason.code}-${reason.resource_id}-${i}`} className={styles.reason}>
                {link ? (
                  <Link to={link} className={styles.reasonLink}>
                    {reason.message}
                  </Link>
                ) : (
                  reason.message
                )}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className={styles.noReasons}>{t("eligibility.panel.noReasons")}</p>
      )}
    </section>
  );
}

function Flag({ active, label }: { active: boolean; label: string }) {
  return (
    <span className={styles.flag} data-active={active}>
      <span className={styles.flagDot} aria-hidden="true" />
      {label}
    </span>
  );
}

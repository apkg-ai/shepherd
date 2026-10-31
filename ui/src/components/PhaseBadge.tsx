import { useTranslation } from "react-i18next";
import type { TaskPhase } from "../api/generated/model/taskPhase";
import styles from "./PhaseBadge.module.css";

const phaseKeys: Record<TaskPhase, string> = {
  planning: "common.phase.planning",
  plan_review: "common.phase.planReview",
  execution: "common.phase.execution",
  work_review: "common.phase.workReview",
  complete: "common.phase.complete",
};

interface PhaseBadgeProps {
  phase: string;
}

export function PhaseBadge({ phase }: PhaseBadgeProps) {
  const { t } = useTranslation();
  const key = phase in phaseKeys ? phaseKeys[phase as TaskPhase] : undefined;
  return (
    <span className={styles.badge} data-phase={phase}>
      {key ? t(key) : phase}
    </span>
  );
}

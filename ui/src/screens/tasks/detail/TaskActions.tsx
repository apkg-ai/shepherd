import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { claimTask } from "../../../api/generated/claims/claims";
import {
  acceptTask,
  blockTask,
  unblockTask,
  cancelTask,
  waiveTask,
  getGetTaskQueryKey,
} from "../../../api/generated/tasks/tasks";
import type { Eligibility } from "../../../api/generated/model/eligibility";
import { newIdempotencyKey } from "../../../lib/idempotency";
import { Button } from "../../../components/Button";
import { ReasonDialog } from "../../../components/ReasonDialog";
import styles from "./TaskActions.module.css";

/**
 * Only render actions present in allowed_actions — never derive from status.
 * allowed_actions carries the REST operationIds emitted by the core
 * eligibility evaluator, so the keys must match that vocabulary exactly.
 * claimTask is phase-agnostic: the server gates it on can_plan/can_execute,
 * and the buttons split the claimed phase on those flags.
 * release / report_success / report_failure are excluded: they require a
 * lease token from the claim response, and the browser claim flow does not
 * yet persist the token across navigation. They will be added when the
 * in-memory lease store is wired (step 017 follow-up).
 */
const ACTION_DEFS = [
  { key: "acceptTask", i18n: "task.action.accept" },
  { key: "blockTask", i18n: "task.action.block" },
  { key: "unblockTask", i18n: "task.action.unblock" },
  { key: "cancelTask", i18n: "task.action.cancel" },
  { key: "waiveTask", i18n: "task.action.waive" },
] as const;

const CLAIM_PHASES = [
  { phase: "plan", busyKey: "claim_plan", i18n: "task.action.claimPlan" },
  { phase: "execute", busyKey: "claim_execute", i18n: "task.action.claimExecute" },
] as const;

type ReasonAction = "blockTask" | "cancelTask" | "waiveTask";

interface TaskActionsProps {
  projectId: string;
  taskId: string;
  revision: number;
  eligibility: Eligibility;
}

export function TaskActions({ projectId, taskId, revision, eligibility }: TaskActionsProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const allowed = new Set(eligibility.allowed_actions);

  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [reasonDialog, setReasonDialog] = useState<ReasonAction | null>(null);

  const invalidateTask = () =>
    queryClient.invalidateQueries({ queryKey: getGetTaskQueryKey(projectId, taskId) });

  const claim = useMutation({
    mutationFn: (phase: "plan" | "execute") =>
      claimTask(
        projectId,
        taskId,
        { phase },
        {
          headers: {
            "If-Match": `"${revision}"`,
            "Idempotency-Key": newIdempotencyKey(),
          },
        },
      ),
    onSettled: () => {
      setBusyAction(null);
      void invalidateTask();
    },
  });

  const accept = useMutation({
    mutationFn: () =>
      acceptTask(
        projectId,
        taskId,
        {},
        {
          headers: {
            "If-Match": `"${revision}"`,
            "Idempotency-Key": newIdempotencyKey(),
          },
        },
      ),
    onSettled: () => {
      setBusyAction(null);
      void invalidateTask();
    },
  });

  const unblock = useMutation({
    mutationFn: () =>
      unblockTask(
        projectId,
        taskId,
        {},
        {
          headers: {
            "If-Match": `"${revision}"`,
            "Idempotency-Key": newIdempotencyKey(),
          },
        },
      ),
    onSettled: () => {
      setBusyAction(null);
      void invalidateTask();
    },
  });

  const block = useMutation({
    mutationFn: (reason: string) =>
      blockTask(
        projectId,
        taskId,
        { reason },
        {
          headers: {
            "If-Match": `"${revision}"`,
            "Idempotency-Key": newIdempotencyKey(),
          },
        },
      ),
    onSettled: () => {
      setBusyAction(null);
      setReasonDialog(null);
      void invalidateTask();
    },
  });

  const cancel = useMutation({
    mutationFn: (reason: string) =>
      cancelTask(
        projectId,
        taskId,
        { reason },
        {
          headers: {
            "If-Match": `"${revision}"`,
            "Idempotency-Key": newIdempotencyKey(),
          },
        },
      ),
    onSettled: () => {
      setBusyAction(null);
      setReasonDialog(null);
      void invalidateTask();
    },
  });

  const waive = useMutation({
    mutationFn: (reason: string) =>
      waiveTask(
        projectId,
        taskId,
        { reason },
        {
          headers: {
            "If-Match": `"${revision}"`,
            "Idempotency-Key": newIdempotencyKey(),
          },
        },
      ),
    onSettled: () => {
      setBusyAction(null);
      setReasonDialog(null);
      void invalidateTask();
    },
  });

  const handleAction = (key: string) => {
    if (busyAction) return;
    setBusyAction(key);

    switch (key) {
      case "claim_plan":
        claim.mutate("plan");
        break;
      case "claim_execute":
        claim.mutate("execute");
        break;
      case "acceptTask":
        accept.mutate();
        break;
      case "unblockTask":
        unblock.mutate();
        break;
      case "blockTask":
        setBusyAction(null);
        setReasonDialog("blockTask");
        break;
      case "cancelTask":
        setBusyAction(null);
        setReasonDialog("cancelTask");
        break;
      case "waiveTask":
        setBusyAction(null);
        setReasonDialog("waiveTask");
        break;
      default:
        setBusyAction(null);
    }
  };

  const handleReasonSubmit = (reason: string) => {
    if (!reasonDialog) return;
    setBusyAction(reasonDialog);
    switch (reasonDialog) {
      case "blockTask":
        block.mutate(reason);
        break;
      case "cancelTask":
        cancel.mutate(reason);
        break;
      case "waiveTask":
        waive.mutate(reason);
        break;
    }
  };

  const visibleClaims = allowed.has("claimTask")
    ? CLAIM_PHASES.filter((c) =>
        c.phase === "plan" ? eligibility.can_plan : eligibility.can_execute,
      )
    : [];
  const visibleActions = ACTION_DEFS.filter((a) => allowed.has(a.key));

  if (visibleActions.length === 0 && visibleClaims.length === 0) {
    return (
      <section className={styles.panel} aria-label={t("task.action.noActions")}>
        <p className={styles.empty}>{t("task.action.noActions")}</p>
      </section>
    );
  }

  const reasonTitles: Record<ReasonAction, string> = {
    blockTask: t("task.action.blockTitle"),
    cancelTask: t("task.action.cancelTitle"),
    waiveTask: t("task.action.waiveTitle"),
  };

  return (
    <>
      <section className={styles.panel}>
        <div className={styles.actions}>
          {visibleClaims.map((claimPhase) => (
            <Button
              key={claimPhase.busyKey}
              variant="secondary"
              busy={busyAction === claimPhase.busyKey}
              disabled={busyAction !== null && busyAction !== claimPhase.busyKey}
              onClick={() => handleAction(claimPhase.busyKey)}
            >
              {t(claimPhase.i18n)}
            </Button>
          ))}
          {visibleActions.map((action) => (
            <Button
              key={action.key}
              variant={
                action.key === "cancelTask" || action.key === "waiveTask" ? "danger" : "secondary"
              }
              busy={busyAction === action.key}
              disabled={busyAction !== null && busyAction !== action.key}
              onClick={() => handleAction(action.key)}
            >
              {t(action.i18n)}
            </Button>
          ))}
        </div>
      </section>

      {reasonDialog ? (
        <ReasonDialog
          open
          title={reasonTitles[reasonDialog]}
          busy={busyAction === reasonDialog}
          onSubmit={handleReasonSubmit}
          onCancel={() => setReasonDialog(null)}
        />
      ) : null}
    </>
  );
}

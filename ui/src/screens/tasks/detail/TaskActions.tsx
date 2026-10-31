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

/** Only render actions present in allowed_actions — never derive from status. */
const ACTION_DEFS = [
  { key: "claim_plan", i18n: "task.action.claimPlan" },
  { key: "claim_execute", i18n: "task.action.claimExecute" },
  { key: "release", i18n: "task.action.release" },
  { key: "report_success", i18n: "task.action.reportSuccess" },
  { key: "report_failure", i18n: "task.action.reportFailure" },
  { key: "accept", i18n: "task.action.accept" },
  { key: "block", i18n: "task.action.block" },
  { key: "unblock", i18n: "task.action.unblock" },
  { key: "cancel", i18n: "task.action.cancel" },
  { key: "waive", i18n: "task.action.waive" },
] as const;

type ReasonAction = "block" | "cancel" | "waive";

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
    meta: { silent: true },
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
    meta: { silent: true },
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
    meta: { silent: true },
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
    meta: { silent: true },
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
    meta: { silent: true },
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
    meta: { silent: true },
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
      case "accept":
        accept.mutate();
        break;
      case "unblock":
        unblock.mutate();
        break;
      case "block":
        setBusyAction(null);
        setReasonDialog("block");
        break;
      case "cancel":
        setBusyAction(null);
        setReasonDialog("cancel");
        break;
      case "waive":
        setBusyAction(null);
        setReasonDialog("waive");
        break;
      default:
        setBusyAction(null);
    }
  };

  const handleReasonSubmit = (reason: string) => {
    if (!reasonDialog) return;
    setBusyAction(reasonDialog);
    switch (reasonDialog) {
      case "block":
        block.mutate(reason);
        break;
      case "cancel":
        cancel.mutate(reason);
        break;
      case "waive":
        waive.mutate(reason);
        break;
    }
  };

  const visibleActions = ACTION_DEFS.filter((a) => allowed.has(a.key));

  if (visibleActions.length === 0) {
    return (
      <section className={styles.panel} aria-label={t("task.action.noActions")}>
        <p className={styles.empty}>{t("task.action.noActions")}</p>
      </section>
    );
  }

  const reasonTitles: Record<ReasonAction, string> = {
    block: t("task.action.blockTitle"),
    cancel: t("task.action.cancelTitle"),
    waive: t("task.action.waiveTitle"),
  };

  return (
    <>
      <section className={styles.panel}>
        <div className={styles.actions}>
          {visibleActions.map((action) => (
            <Button
              key={action.key}
              variant={action.key === "cancel" || action.key === "waive" ? "danger" : "secondary"}
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

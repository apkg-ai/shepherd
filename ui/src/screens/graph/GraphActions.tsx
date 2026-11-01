import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { useMutation } from "@tanstack/react-query";
import type { GraphNodesItem } from "../../api/generated/model/graphNodesItem";
import { createDependency, deleteDependency } from "../../api/generated/dependencies/dependencies";
import { newIdempotencyKey } from "../../lib/idempotency";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import styles from "./GraphActions.module.css";

interface GraphActionsProps {
  projectId: string;
  level: "epic" | "task";
  nodes: GraphNodesItem[];
  selectedNodeId: string | null;
  addDialogOpen: boolean;
  onAddDialogClose: () => void;
  removeEdge: { dependencyId: string; dependentTitle: string; prerequisiteTitle: string } | null;
  onRemoveDialogClose: () => void;
  onSuccess: () => void;
}

export function GraphActions({
  projectId,
  level,
  nodes,
  selectedNodeId,
  addDialogOpen,
  onAddDialogClose,
  removeEdge,
  onRemoveDialogClose,
  onSuccess,
}: GraphActionsProps) {
  const { t } = useTranslation();
  const [prerequisiteId, setPrerequisiteId] = useState("");

  const selectedNode = nodes.find((n) => n.id === selectedNodeId);
  const candidates = nodes.filter((n) => n.id !== selectedNodeId);

  const createMutation = useMutation({
    mutationFn: (vars: { dependentId: string; prerequisiteId: string }) =>
      createDependency(
        projectId,
        { level, dependent_id: vars.dependentId, prerequisite_id: vars.prerequisiteId },
        { headers: { "Idempotency-Key": newIdempotencyKey() } },
      ),
    onSuccess: () => {
      onSuccess();
      onAddDialogClose();
      setPrerequisiteId("");
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (dependencyId: string) =>
      deleteDependency(projectId, dependencyId, {
        headers: { "Idempotency-Key": newIdempotencyKey() },
      }),
    onSuccess: () => {
      onSuccess();
      onRemoveDialogClose();
    },
  });

  const handleAddSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (!selectedNodeId || !prerequisiteId) return;
    createMutation.mutate({ dependentId: selectedNodeId, prerequisiteId });
  };

  const handleRemoveConfirm = () => {
    if (!removeEdge) return;
    deleteMutation.mutate(removeEdge.dependencyId);
  };

  const handleAddClose = () => {
    onAddDialogClose();
    setPrerequisiteId("");
    createMutation.reset();
  };

  const handleRemoveClose = () => {
    onRemoveDialogClose();
    deleteMutation.reset();
  };

  return (
    <>
      <Dialog
        open={addDialogOpen}
        title={t("graph.action.addDependencyTitle")}
        onClose={handleAddClose}
      >
        <form onSubmit={handleAddSubmit} className={styles.form}>
          <div>
            <div className={styles.label}>{t("graph.action.addDependencyDependent")}</div>
            <div className={styles.selectedNode}>{selectedNode?.title}</div>
          </div>
          <div>
            <label className={styles.label} htmlFor="prerequisite-select">
              {t("graph.action.addDependencyPrompt")}
            </label>
            <select
              id="prerequisite-select"
              className={styles.select}
              value={prerequisiteId}
              onChange={(e) => setPrerequisiteId(e.target.value)}
            >
              <option value="" disabled>
                —
              </option>
              {candidates.map((node) => (
                <option key={node.id} value={node.id}>
                  {node.title}
                </option>
              ))}
            </select>
          </div>
          <div className={styles.actions}>
            <Button
              type="submit"
              variant="primary"
              busy={createMutation.isPending}
              disabled={!prerequisiteId}
            >
              {t("graph.action.addDependencySubmit")}
            </Button>
          </div>
        </form>
      </Dialog>

      <Dialog
        open={removeEdge !== null}
        title={t("graph.action.removeDependencyTitle")}
        onClose={handleRemoveClose}
      >
        <p className={styles.warning}>{t("graph.action.removeDependencyWarning")}</p>
        {removeEdge && (
          <p className={styles.edgeInfo}>
            {removeEdge.prerequisiteTitle} <span aria-hidden="true">→</span>
            <span className={styles.srOnly}>{t("graph.action.removeDependencyArrow")}</span>{" "}
            {removeEdge.dependentTitle}
          </p>
        )}
        <div className={styles.actions}>
          <Button variant="danger" busy={deleteMutation.isPending} onClick={handleRemoveConfirm}>
            {t("graph.action.removeDependencyConfirm")}
          </Button>
        </div>
      </Dialog>
    </>
  );
}

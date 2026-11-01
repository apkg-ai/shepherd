import { useState } from "react";
import { useNavigate } from "react-router";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { errorSlug, isShepherdError, fieldErrors, type ShepherdError } from "../api/problem";
import { newIdempotencyKey } from "./idempotency";

export interface ConflictState {
  localTitle: string;
  localDescription: string;
  remoteTitle: string;
  remoteDescription: string;
  remoteRevision: number;
}

function isIdempotencyConflict(error: ShepherdError): boolean {
  return error.status === 409 && errorSlug(error) === "idempotency_conflict";
}

interface UseEntityFormOptions<TEntity> {
  /** Populated in edit mode, undefined for create. */
  entityId?: string;
  initialTitle: string;
  initialDescription: string;
  initialRevision: number;

  createFn?: (
    data: Record<string, unknown>,
    opts: { headers: Record<string, string> },
  ) => Promise<TEntity>;
  onCreateSuccess?: (entity: TEntity) => void;

  updateFn?: (
    data: Record<string, unknown>,
    opts: { headers: Record<string, string> },
  ) => Promise<TEntity>;
  /** Direct fetch for conflict resolution — bypasses the mounted query. */
  refetchFn?: () => Promise<TEntity>;
  onUpdateSuccess?: () => void;

  /** Extract fields from a freshly-fetched entity for conflict comparison. */
  getTitle: (entity: TEntity) => string;
  getDescription: (entity: TEntity) => string;
  getRevision: (entity: TEntity) => number;

  /** Query keys to invalidate on success. */
  listQueryKey: readonly unknown[];
  detailQueryKey?: readonly unknown[];

  /** Where Cancel navigates (create → parent, edit → detail). */
  createBackPath: string;
  editBackPath?: string;

  /** i18n key for idempotency-conflict explanation in create mode. */
  conflictInspectKey?: string;
  /** Link path for inspecting conflicted resources in create mode. */
  conflictInspectPath?: string;
}

export function useEntityForm<TEntity>(options: UseEntityFormOptions<TEntity>) {
  const {
    entityId,
    initialTitle,
    initialDescription,
    initialRevision,
    createFn,
    onCreateSuccess,
    updateFn,
    refetchFn,
    onUpdateSuccess,
    getTitle,
    getDescription,
    getRevision,
    listQueryKey,
    detailQueryKey,
    createBackPath,
    editBackPath,
    conflictInspectKey,
    conflictInspectPath,
  } = options;

  const isEdit = Boolean(entityId);
  const { t } = useTranslation();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [title, setTitle] = useState(initialTitle);
  const [description, setDescription] = useState(initialDescription);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [createConflict, setCreateConflict] = useState(false);
  const [idempotencyKey, setIdempotencyKey] = useState(newIdempotencyKey);
  const [revision, setRevision] = useState(initialRevision);
  const [dirty, setDirtyState] = useState(false);
  const [conflict, setConflict] = useState<ConflictState | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);

  const setDirty = () => setDirtyState(true);

  const create = useMutation<TEntity, unknown, Record<string, unknown>>({
    mutationFn: (vars) => createFn!(vars, { headers: { "Idempotency-Key": idempotencyKey } }),
    meta: { silent: true },
    onSuccess: (data) => {
      void queryClient.invalidateQueries({ queryKey: listQueryKey });
      onCreateSuccess?.(data);
    },
    onError: (error) => {
      if (isShepherdError(error) && error.status === 422) {
        setIdempotencyKey(newIdempotencyKey());
        setCreateConflict(false);
        setErrors(fieldErrors(error));
      } else if (isShepherdError(error) && isIdempotencyConflict(error)) {
        void queryClient.invalidateQueries({ queryKey: listQueryKey });
        setCreateConflict(true);
        setErrors({
          _form: conflictInspectKey ? t(conflictInspectKey) : t("common.error.unexpected"),
        });
      } else {
        setCreateConflict(false);
        setErrors({
          _form: isShepherdError(error) ? error.title : t("common.error.unexpected"),
        });
      }
    },
  });

  const update = useMutation<TEntity, unknown, { payload: Record<string, unknown>; rev: number }>({
    mutationFn: (vars) =>
      updateFn!(vars.payload, {
        headers: {
          "If-Match": `"${vars.rev}"`,
          "Idempotency-Key": idempotencyKey,
        },
      }),
    meta: { silent: true },
    onSuccess: () => {
      if (detailQueryKey) {
        void queryClient.invalidateQueries({ queryKey: detailQueryKey });
      }
      void queryClient.invalidateQueries({ queryKey: listQueryKey });
      onUpdateSuccess?.();
    },
    onError: async (error) => {
      if (isShepherdError(error) && (error.status === 412 || isIdempotencyConflict(error))) {
        try {
          const fresh = await refetchFn!();
          if (detailQueryKey) {
            queryClient.setQueryData(detailQueryKey, fresh);
          }
          setConflict({
            localTitle: title,
            localDescription: description,
            remoteTitle: getTitle(fresh),
            remoteDescription: getDescription(fresh),
            remoteRevision: getRevision(fresh),
          });
        } catch {
          setErrors({ _form: t("common.error.unexpected") });
        }
      } else if (isShepherdError(error) && error.status === 422) {
        setIdempotencyKey(newIdempotencyKey());
        setErrors(fieldErrors(error));
      } else {
        setErrors({
          _form: isShepherdError(error) ? error.title : t("common.error.unexpected"),
        });
      }
    },
  });

  const isPending = create.isPending || update.isPending;

  const submitCreate = (vars: Record<string, unknown>) => {
    setErrors({});
    create.mutate(vars);
  };

  const submitUpdate = (vars: Record<string, unknown>) => {
    setErrors({});
    update.mutate({ payload: vars, rev: revision });
  };

  const handleConflictRetry = () => {
    if (!conflict) return;
    setIdempotencyKey(newIdempotencyKey());
    setRevision(conflict.remoteRevision);
    setConflict(null);
  };

  const handleConflictDiscard = () => {
    setConflict(null);
    if (editBackPath) {
      void navigate(editBackPath, { replace: true });
    }
  };

  const navigateBack = () => {
    if (isEdit && editBackPath) {
      void navigate(editBackPath);
    } else {
      void navigate(createBackPath);
    }
  };

  const handleCancel = () => {
    if (dirty) {
      setConfirmDiscard(true);
    } else {
      navigateBack();
    }
  };

  return {
    title,
    setTitle,
    description,
    setDescription,
    errors,
    setErrors,
    dirty,
    setDirty,
    isPending,
    isEdit,
    conflict,
    createConflict,
    confirmDiscard,
    setConfirmDiscard,
    conflictInspectPath,

    submitCreate,
    submitUpdate,
    handleConflictRetry,
    handleConflictDiscard,
    handleCancel,
    navigateBack,
  };
}

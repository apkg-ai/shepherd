-- At most one active claim per task, enforced at the DB level (invariant 3).
-- Active = not yet released; expired-but-unswept rows still occupy the slot,
-- so the claim path must release them before inserting a new claim.
CREATE UNIQUE INDEX idx_claims_one_active
    ON claims(task_id)
    WHERE released_at IS NULL;

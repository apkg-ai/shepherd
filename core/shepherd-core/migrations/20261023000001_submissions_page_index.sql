-- Keyset pagination for the submissions list: submissions_queue leads with
-- status/policy/kind, so the task history and project pages scanned with a
-- temp B-tree sort. Reviews are served by the UNIQUE(submission_id) index.
CREATE INDEX submissions_task_page ON submissions(task_id,created_at,id);

CREATE INDEX epics_project ON epics(project_id,created_at,id);

CREATE INDEX task_deps_project ON task_dependencies(project_id,created_at,id);

CREATE INDEX epic_deps_project ON epic_dependencies(project_id,created_at,id);

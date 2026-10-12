-- Keyset pagination for the document lists: documents_owner leads with
-- owner_kind/owner_id, so an unfiltered project page needed a temp B-tree
-- sort, and document_revisions had no page index at all.
CREATE INDEX documents_project_page ON documents(project_id,created_at,id);

CREATE INDEX document_revisions_page ON document_revisions(document_id,created_at,id);

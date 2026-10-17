-- Keyset pagination for the documents list: documents_owner leads with
-- owner_kind/owner_id, so an unfiltered project page needed a temp B-tree
-- sort. The revisions list orders by number and is served by the
-- UNIQUE(document_id,number) autoindex.
CREATE INDEX documents_project_page ON documents(project_id,created_at,id);

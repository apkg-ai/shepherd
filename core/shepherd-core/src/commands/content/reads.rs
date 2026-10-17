// Document reads: project-scoped gets and cursor-bound lists (plan/07).
use super::*;

impl Store {
    pub async fn get_document(
        &self,
        project: &ProjectId,
        id: &DocumentId,
    ) -> Result<Document, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = document_row(&mut tx, project, id).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn get_document_revision(
        &self,
        project: &ProjectId,
        id: &DocumentRevisionId,
    ) -> Result<DocumentRevision, DomainError> {
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        let found = revision_row(&mut tx, project, id).await?;
        tx.commit().await.map_err(StorageError::from)?;
        found.ok_or(DomainError::NotFound)
    }

    pub async fn list_documents(
        &self,
        project: &ProjectId,
        filters: &DocumentListFilters,
        params: &ListParams,
    ) -> Result<Page<Document>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown project is 404.
        if !project_exists(&mut tx, project).await? {
            return Err(DomainError::NotFound);
        }
        // An unknown owner-filter target is 404 like listTasks' epic filter.
        if let Some(owner_id) = &filters.owner_id
            && !owner_filter_exists(&mut tx, project, filters.owner_kind, owner_id).await?
        {
            return Err(DomainError::NotFound);
        }
        let filter = documents_filter(project, filters);
        let after = decode_after(params, "listDocuments", &filter)?;

        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {DOCUMENT_COLUMNS} FROM documents WHERE project_id = "
        ));
        builder.push_bind(project.to_string());
        if let Some(owner_kind) = &filters.owner_kind {
            builder
                .push(" AND owner_kind = ")
                .push_bind(owner_kind.as_str());
        }
        if let Some(owner_id) = &filters.owner_id {
            builder
                .push(" AND owner_id = ")
                .push_bind(owner_id.to_string());
        }
        if let Some(kind) = &filters.kind {
            builder.push(" AND kind = ").push_bind(kind.as_str());
        }
        push_keyset_tail(&mut builder, " AND ", &after, limit);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(document_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |document: &Document| {
            encode_cursor(
                "listDocuments",
                &filter,
                &format_ts(&document.created_at),
                &document.id.to_string(),
            )
        }))
    }

    pub async fn list_document_revisions(
        &self,
        project: &ProjectId,
        document: &DocumentId,
        params: &ListParams,
    ) -> Result<Page<DocumentRevision>, DomainError> {
        let limit = effective_limit(params)?;
        let mut tx = self.pool().begin().await.map_err(StorageError::from)?;
        // Route membership before cursor validity (plan/04): unknown document is 404.
        if !document_exists(&mut tx, project, document).await? {
            return Err(DomainError::NotFound);
        }
        let filter = format!("project={project}&document={document}");
        let after = decode_after(params, "listDocumentRevisions", &filter)?;

        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {REVISION_COLUMNS} FROM document_revisions WHERE document_id = "
        ));
        builder.push_bind(document.to_string());
        // The seek key is the unique, strictly ordered revision number, resolved
        // from the cursor's row id in the same statement. The inner select
        // cannot miss: the cursor MAC binds endpoint and filter, and the filter
        // embeds the document id.
        if let Some((_, id)) = &after {
            builder
                .push(" AND number < (SELECT number FROM document_revisions WHERE id = ")
                .push_bind(id.clone())
                .push(" AND document_id = ")
                .push_bind(document.to_string())
                .push(")");
        }
        // Revisions list newest first (plan/07).
        builder
            .push(" ORDER BY number DESC LIMIT ")
            .push_bind(limit + 1);

        let rows = builder.build().fetch_all(&mut *tx).await?;
        tx.commit().await.map_err(StorageError::from)?;
        let items = rows
            .iter()
            .map(revision_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(split_page(items, limit, |revision: &DocumentRevision| {
            encode_cursor(
                "listDocumentRevisions",
                &filter,
                &format_ts(&revision.created_at),
                &revision.id.to_string(),
            )
        }))
    }
}

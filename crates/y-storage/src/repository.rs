//! Generic repository patterns and base helpers.

use sqlx::SqlitePool;

/// A generic pagination request.
#[derive(Debug, Clone)]
pub struct Pagination {
    /// Number of items to skip.
    pub offset: u64,
    /// Maximum number of items to return.
    pub limit: u64,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 100,
        }
    }
}

/// A paginated result.
#[derive(Debug, Clone)]
pub struct PaginatedResult<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

impl<T> PaginatedResult<T> {
    /// Whether there are more items after this page.
    pub fn has_more(&self) -> bool {
        self.offset + self.limit < self.total
    }
}

/// Helper to get total row count for a table.
pub async fn count_rows(
    pool: &SqlitePool,
    table: &str,
    where_clause: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let sql = if let Some(clause) = where_clause {
        format!("SELECT COUNT(*) as cnt FROM {table} WHERE {clause}")
    } else {
        format!("SELECT COUNT(*) as cnt FROM {table}")
    };

    let row: (i64,) = sqlx::query_as(&sql).fetch_one(pool).await?;

    Ok(u64::try_from(row.0).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagination_default() {
        let p = Pagination::default();
        assert_eq!(p.offset, 0);
        assert_eq!(p.limit, 100);
    }

    #[test]
    fn test_paginated_result_has_more() {
        let result = PaginatedResult {
            items: vec![1, 2, 3],
            total: 10,
            offset: 0,
            limit: 3,
        };
        assert!(result.has_more());

        let last_page = PaginatedResult {
            items: vec![10],
            total: 10,
            offset: 9,
            limit: 3,
        };
        assert!(!last_page.has_more());
    }
}
